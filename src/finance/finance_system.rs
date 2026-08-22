//! Financial validation and atomic ledger commits; sibling finance state owns balances and indexes.

use crate::core::entity::EntityRef;
use crate::core::id::{FinancialAccountId, IdExhaustionError, LedgerTransactionId, MandateId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    ensure_mandate_authority_current, resolve_mandate_authority, DelegationError,
};
use crate::delegation::{MandateStatus, ResolvedMandateAuthority};
use crate::finance::{
    build_budget_usage, BudgetUsageRecord, FinancialAccountDraft, FinancialAccountRecord,
    LedgerTransactionDraft, LedgerTransactionRecord, Money,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FinanceError {
    #[error("ledger transaction memo must not be empty")]
    EmptyMemo,
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("ledger transaction must contain at least two postings")]
    TooFewPostings,
    #[error("ledger transaction repeats account {0}")]
    DuplicateAccount(FinancialAccountId),
    #[error("ledger transaction contains a zero-value posting for account {0}")]
    ZeroPosting(FinancialAccountId),
    #[error("ledger transaction postings do not balance to zero; net cents {net_cents}")]
    Unbalanced { net_cents: i64 },
    #[error("ledger transaction posting sum overflowed the balance accumulator")]
    PostingSumOverflow,
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error("ledger transaction would overflow account {0}")]
    BalanceOverflow(FinancialAccountId),
    #[error("ledger transaction cannot occur in the future")]
    OccursInFuture,
    #[error("financial account {account} changed after validation; expected version {expected}, found {found}")]
    StaleAccount {
        account: FinancialAccountId,
        expected: u32,
        found: u32,
    },
    #[error("mandate {0} does not exist")]
    MissingMandate(MandateId),
    #[error("mandate {0} is not active")]
    InactiveMandate(MandateId),
    #[error("delegated authority is invalid: {0}")]
    Delegation(#[from] DelegationError),
    #[error("mandate {0} has no budget authority")]
    MissingBudget(MandateId),
    #[error("mandate {mandate} budget requires posting from funding account {account}")]
    MissingBudgetOutflow {
        mandate: MandateId,
        account: FinancialAccountId,
    },
    #[error("mandate {mandate} budget posting from account {account} must be an outflow")]
    InvalidBudgetOutflow {
        mandate: MandateId,
        account: FinancialAccountId,
    },
    #[error("budget arithmetic overflow for mandate {0}")]
    BudgetOverflow(MandateId),
    #[error("mandate {mandate} budget exceeded: limit {limit_cents} cents, used {used_cents}, requested {requested_cents}")]
    BudgetExceeded {
        mandate: MandateId,
        limit_cents: i64,
        used_cents: i64,
        requested_cents: i64,
    },
}

pub fn insert_account(
    state: &mut AppState,
    draft: FinancialAccountDraft,
) -> Result<FinancialAccountId, FinanceError> {
    if !crate::core::entity::is_entity_present(state, draft.owner.entity()) {
        return Err(FinanceError::MissingEntity(draft.owner.entity()));
    }
    let id = state.ids.next_financial_account()?;
    state.finance.insert_account(FinancialAccountRecord {
        id,
        owner: draft.owner,
        kind: draft.kind,
        balance: Money::ZERO,
        version: 1,
    });
    Ok(id)
}

pub struct ValidatedLedgerTransaction {
    draft: LedgerTransactionDraft,
    balances: BTreeMap<FinancialAccountId, Money>,
    expected_versions: BTreeMap<FinancialAccountId, u32>,
    budget_usage: Option<BudgetUsageRecord>,
    authority_snapshot: Option<ResolvedMandateAuthority>,
}

impl ValidatedLedgerTransaction {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, FinanceError> {
        for (account, expected) in &self.expected_versions {
            let record = state
                .finance
                .get_account(*account)
                .ok_or(FinanceError::MissingAccount(*account))?;
            if record.version() != *expected {
                return Err(FinanceError::StaleAccount {
                    account: *account,
                    expected: *expected,
                    found: record.version(),
                });
            }
        }
        if let Some(snapshot) = self.authority_snapshot {
            ensure_mandate_authority_current(state, snapshot)?;
        }
        if let Some(usage) = self.budget_usage {
            let summary = resolve_budget_usage(state, usage.mandate(), self.draft.occurred_at)?;
            let next_used = summary
                .used
                .checked_add(usage.amount())
                .ok_or(FinanceError::BudgetOverflow(usage.mandate()))?;
            if next_used > summary.limit {
                return Err(FinanceError::BudgetExceeded {
                    mandate: usage.mandate(),
                    limit_cents: summary.limit.cents(),
                    used_cents: summary.used.cents(),
                    requested_cents: usage.amount().cents(),
                });
            }
        }
        let id = state.ids.next_ledger_transaction()?;
        let LedgerTransactionDraft {
            occurred_at,
            memo,
            postings,
            authorization: _,
        } = self.draft;
        state.finance.apply_transaction(
            LedgerTransactionRecord {
                id,
                occurred_at,
                memo,
                postings,
                budget_usage: self.budget_usage,
            },
            &self.balances,
        );
        Ok(id)
    }
}

pub fn validate_record_transaction(
    state: &AppState,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    if draft.memo.trim().is_empty() {
        return Err(FinanceError::EmptyMemo);
    }
    if draft.postings.len() < 2 {
        return Err(FinanceError::TooFewPostings);
    }
    if draft.occurred_at > state.now() {
        return Err(FinanceError::OccursInFuture);
    }

    let mut seen = BTreeSet::new();
    let mut net_cents: i128 = 0;
    let mut balances = BTreeMap::new();
    let mut expected_versions = BTreeMap::new();
    for posting in &draft.postings {
        if !seen.insert(posting.account) {
            return Err(FinanceError::DuplicateAccount(posting.account));
        }
        if posting.amount == Money::ZERO {
            return Err(FinanceError::ZeroPosting(posting.account));
        }
        net_cents = net_cents
            .checked_add(i128::from(posting.amount.cents()))
            .ok_or(FinanceError::PostingSumOverflow)?;
        if net_cents > i128::from(i64::MAX) || net_cents < i128::from(i64::MIN) {
            return Err(FinanceError::PostingSumOverflow);
        }
        let account = state
            .finance
            .get_account(posting.account)
            .ok_or(FinanceError::MissingAccount(posting.account))?;
        let next = account
            .balance()
            .checked_add(posting.amount)
            .ok_or(FinanceError::BalanceOverflow(posting.account))?;
        balances.insert(posting.account, next);
        expected_versions.insert(posting.account, account.version());
    }
    if net_cents != 0 {
        let diagnostic =
            i64::try_from(net_cents).unwrap_or(if net_cents > 0 { i64::MAX } else { i64::MIN });
        return Err(FinanceError::Unbalanced {
            net_cents: diagnostic,
        });
    }
    let budget_validation = resolve_transaction_budget(state, &draft)?;
    Ok(ValidatedLedgerTransaction {
        draft,
        balances,
        expected_versions,
        budget_usage: budget_validation.usage,
        authority_snapshot: budget_validation.authority_snapshot,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetUsageSummary {
    pub mandate: MandateId,
    pub period_start: crate::core::time::SimTime,
    pub period_end: crate::core::time::SimTime,
    pub limit: Money,
    pub used: Money,
    pub remaining: Money,
}

pub fn resolve_budget_usage(
    state: &AppState,
    mandate: MandateId,
    at: crate::core::time::SimTime,
) -> Result<BudgetUsageSummary, FinanceError> {
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(FinanceError::MissingMandate(mandate))?;
    if record.status() != MandateStatus::Active {
        return Err(FinanceError::InactiveMandate(mandate));
    }
    let budget = record
        .budget()
        .ok_or(FinanceError::MissingBudget(mandate))?;
    let window = budget.period.window(at);
    let mut used = Money::ZERO;
    for transaction in state.finance.transactions_for_mandate(mandate) {
        if let Some(usage) = transaction.budget_usage() {
            if usage.period_start() == window.start() && usage.period_end() == window.end() {
                used = used
                    .checked_add(usage.amount())
                    .ok_or(FinanceError::BudgetOverflow(mandate))?;
            }
        }
    }
    let remaining = budget
        .limit
        .checked_sub(used)
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    Ok(BudgetUsageSummary {
        mandate,
        period_start: window.start(),
        period_end: window.end(),
        limit: budget.limit,
        used,
        remaining,
    })
}

struct ResolvedBudgetValidation {
    usage: Option<BudgetUsageRecord>,
    authority_snapshot: Option<ResolvedMandateAuthority>,
}

fn resolve_transaction_budget(
    state: &AppState,
    draft: &LedgerTransactionDraft,
) -> Result<ResolvedBudgetValidation, FinanceError> {
    let Some(authorization) = draft.authorization else {
        return Ok(ResolvedBudgetValidation {
            usage: None,
            authority_snapshot: None,
        });
    };
    let mandate = authorization.mandate;
    let authority_snapshot = resolve_mandate_authority(state, authorization)?;
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(FinanceError::MissingMandate(mandate))?;
    let budget = record
        .budget()
        .ok_or(FinanceError::MissingBudget(mandate))?;
    let posting = draft
        .postings
        .iter()
        .find(|posting| posting.account == budget.funding_account)
        .ok_or(FinanceError::MissingBudgetOutflow {
            mandate,
            account: budget.funding_account,
        })?;
    if posting.amount.cents() >= 0 {
        return Err(FinanceError::InvalidBudgetOutflow {
            mandate,
            account: budget.funding_account,
        });
    }
    let requested_cents = posting
        .amount
        .cents()
        .checked_neg()
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    let requested = Money::from_cents(requested_cents);
    let summary = resolve_budget_usage(state, mandate, draft.occurred_at)?;
    let next_used = summary
        .used
        .checked_add(requested)
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    if next_used > summary.limit {
        return Err(FinanceError::BudgetExceeded {
            mandate,
            limit_cents: summary.limit.cents(),
            used_cents: summary.used.cents(),
            requested_cents,
        });
    }
    Ok(ResolvedBudgetValidation {
        usage: Some(build_budget_usage(
            authorization,
            authority_snapshot.mandate_version(),
            budget.funding_account,
            summary.period_start,
            summary.period_end,
            requested,
        )),
        authority_snapshot: Some(authority_snapshot),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::delegation::delegation_system::{
        validate_assign_mandate, validate_revise_mandate, DelegationError, MandateRevisionDraft,
    };
    use crate::delegation::{
        BudgetAuthority, BudgetPeriod, MandateAuthority, MandateDraft, ResponsibilityFunction,
        ResponsibilityScope,
    };
    use crate::finance::{
        AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    };
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    fn make_test_budget() -> (
        AppState,
        MandateAuthority,
        FinancialAccountId,
        FinancialAccountId,
    ) {
        let registry = build_registry();
        let mut state = AppState::new(53);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Budget Test Organization".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("organization fixture should validate");
        let manager = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Budget Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("manager fixture should validate");
        let owner = FinancialOwner::Organization(organization);
        let funding = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::AccountedFunds,
            },
        )
        .expect("funding account should validate");
        let destination = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::LegitimateOperating,
            },
        )
        .expect("destination account should validate");
        let mandate = validate_assign_mandate(
            &registry,
            &state,
            MandateDraft {
                organization,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Finance,
                )]),
                standing_orders: BTreeMap::new(),
                budget: Some(BudgetAuthority {
                    funding_account: funding,
                    limit: Money::from_cents(2_500),
                    period: BudgetPeriod::Weekly,
                }),
            },
        )
        .expect("mandate fixture should validate")
        .commit(&mut state)
        .expect("validated mandate should remain current");
        (
            state,
            MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Finance),
            },
            funding,
            destination,
        )
    }

    #[test]
    fn balanced_transaction_commits_all_account_balances_atomically() {
        let registry = build_registry();
        let mut state = AppState::new(31);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Ledger Test".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let owner = FinancialOwner::Organization(organization);
        let street = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::StreetCash,
            },
        )
        .expect("street cash fixture should validate");
        let concealed = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::ConcealedCash,
            },
        )
        .expect("concealed cash fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::Settlement,
            },
        )
        .expect("concealed cash fixture should validate");

        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Opening cash position".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-10_000),
                    },
                    LedgerPosting {
                        account: street,
                        amount: Money::from_cents(10_000),
                    },
                ],
                authorization: None,
            },
        )
        .expect("opening position should validate")
        .commit(&mut state)
        .expect("opening position commit should remain current");

        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Move cash to safe".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: street,
                        amount: Money::from_cents(-2_500),
                    },
                    LedgerPosting {
                        account: concealed,
                        amount: Money::from_cents(2_500),
                    },
                ],
                authorization: None,
            },
        )
        .expect("balanced transfer should validate")
        .commit(&mut state)
        .expect("balanced transfer commit should remain current");

        assert_eq!(
            state
                .finance()
                .get_account(street)
                .expect("street account should exist")
                .balance(),
            Money::from_cents(7_500)
        );
        assert_eq!(
            state
                .finance()
                .get_account(concealed)
                .expect("safe account should exist")
                .balance(),
            Money::from_cents(2_500)
        );
        validate_invariants(&state);
    }

    #[test]
    fn validated_budget_transaction_remains_valid_when_hierarchy_change_is_blocked() {
        let registry = build_registry();
        let (mut state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        let transaction = validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Pending manager-authorized allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-500),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(500),
                    },
                ],
                authorization: Some(authorization),
            },
        )
        .expect("transaction should validate against the current manager snapshot");
        let organization = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should exist")
            .organization();
        let supervisor = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Budget Supervisor".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("supervisor fixture should validate");
        let error = validate_reassign_character(
            &state,
            authorization.manager,
            Some(organization),
            Some(supervisor),
        )
        .expect_err("active mandate must prevent same-organization supervisor reassignment");
        assert_eq!(
            error,
            WorldError::ActiveMandateAssignment {
                character: authorization.manager,
                mandate,
            }
        );
        transaction
            .commit(&mut state)
            .expect("blocked hierarchy change leaves mandate snapshot still valid");
        assert_eq!(
            state
                .finance()
                .get_account(funding)
                .expect("funding account should exist")
                .balance(),
            Money::from_cents(-500)
        );
        assert_eq!(
            state
                .finance()
                .get_account(destination)
                .expect("destination account should exist")
                .balance(),
            Money::from_cents(500)
        );
        validate_invariants(&state);
    }

    #[test]
    fn unbalanced_transaction_leaves_balances_unchanged() {
        let registry = build_registry();
        let mut state = AppState::new(37);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Ledger Test".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let owner = FinancialOwner::Organization(organization);
        let street = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::StreetCash,
            },
        )
        .expect("street cash fixture should validate");
        let concealed = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::ConcealedCash,
            },
        )
        .expect("concealed cash fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::Settlement,
            },
        )
        .expect("concealed cash fixture should validate");

        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Opening cash position".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-10_000),
                    },
                    LedgerPosting {
                        account: street,
                        amount: Money::from_cents(10_000),
                    },
                ],
                authorization: None,
            },
        )
        .expect("opening position should validate")
        .commit(&mut state)
        .expect("opening position commit should remain current");

        let error = match validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Broken transfer".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: street,
                        amount: Money::from_cents(-2_500),
                    },
                    LedgerPosting {
                        account: concealed,
                        amount: Money::from_cents(2_400),
                    },
                ],
                authorization: None,
            },
        ) {
            Ok(_) => panic!("unbalanced transfer must fail before mutation"),
            Err(error) => error,
        };

        assert_eq!(error, FinanceError::Unbalanced { net_cents: -100 });
        assert_eq!(
            state
                .finance()
                .get_account(street)
                .expect("street account should exist")
                .balance(),
            Money::from_cents(10_000)
        );
        assert_eq!(
            state
                .finance()
                .get_account(concealed)
                .expect("safe account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&state);
    }

    #[test]
    fn stale_validated_transaction_cannot_overwrite_newer_balances() {
        let registry = build_registry();
        let mut state = AppState::new(41);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Ledger Test".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let owner = FinancialOwner::Organization(organization);
        let first = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::StreetCash,
            },
        )
        .expect("first account fixture should validate");
        let second = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::ConcealedCash,
            },
        )
        .expect("second account fixture should validate");

        let make_draft = |amount: i64| LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Concurrent transfer".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: first,
                    amount: Money::from_cents(-amount),
                },
                LedgerPosting {
                    account: second,
                    amount: Money::from_cents(amount),
                },
            ],
            authorization: None,
        };

        let stale = validate_record_transaction(&state, make_draft(100))
            .expect("first transaction should validate");
        let current = validate_record_transaction(&state, make_draft(200))
            .expect("second transaction should validate");
        current
            .commit(&mut state)
            .expect("current transaction should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale transaction must not overwrite newer balances");
        assert_eq!(
            error,
            FinanceError::StaleAccount {
                account: first,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .finance()
                .get_account(first)
                .expect("first account should exist")
                .balance(),
            Money::from_cents(-200)
        );
        assert_eq!(
            state
                .finance()
                .get_account(second)
                .expect("second account should exist")
                .balance(),
            Money::from_cents(200)
        );
        validate_invariants(&state);
    }

    #[test]
    fn mandate_budget_usage_is_derived_from_ledger_and_enforced() {
        let (mut state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Delegated operating allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-1_500),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(1_500),
                    },
                ],
                authorization: Some(authorization),
            },
        )
        .expect("transaction within delegated budget should validate")
        .commit(&mut state)
        .expect("validated transaction should remain current");

        let usage = resolve_budget_usage(&state, mandate, state.now())
            .expect("active mandate budget usage should resolve");
        assert_eq!(usage.limit, Money::from_cents(2_500));
        assert_eq!(usage.used, Money::from_cents(1_500));
        assert_eq!(usage.remaining, Money::from_cents(1_000));

        let error = match validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Over-budget allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-1_100),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(1_100),
                    },
                ],
                authorization: Some(authorization),
            },
        ) {
            Ok(_) => panic!("transaction exceeding delegated budget must fail validation"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            FinanceError::BudgetExceeded {
                mandate,
                limit_cents: 2_500,
                used_cents: 1_500,
                requested_cents: 1_100,
            }
        );
        assert_eq!(
            state
                .finance()
                .get_account(funding)
                .expect("funding account should exist")
                .balance(),
            Money::from_cents(-1_500)
        );
        assert_eq!(
            state
                .finance()
                .get_account(destination)
                .expect("destination account should exist")
                .balance(),
            Money::from_cents(1_500)
        );
        validate_invariants(&state);
    }

    #[test]
    fn validated_budget_transaction_becomes_stale_after_mandate_revision() {
        let registry = build_registry();
        let (mut state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        let transaction = validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Pending delegated allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-500),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(500),
                    },
                ],
                authorization: Some(authorization),
            },
        )
        .expect("transaction should validate against current mandate");
        let mandate_record = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should exist");
        let current_budget = mandate_record.budget().expect("mandate should have budget");
        let revision = MandateRevisionDraft {
            scopes: mandate_record.scopes().clone(),
            standing_orders: mandate_record.standing_orders().clone(),
            budget: Some(BudgetAuthority {
                funding_account: current_budget.funding_account,
                limit: Money::from_cents(3_000),
                period: current_budget.period,
            }),
        };
        validate_revise_mandate(&registry, &state, mandate, revision)
            .expect("mandate revision should validate")
            .commit(&mut state)
            .expect("mandate revision should commit");

        let error = transaction
            .commit(&mut state)
            .expect_err("transaction validated against old authority must be stale");
        assert_eq!(
            error,
            FinanceError::Delegation(DelegationError::StaleMandate {
                mandate,
                expected: 1,
                found: 2,
            })
        );
        assert_eq!(
            state
                .finance()
                .get_account(funding)
                .expect("funding account should exist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            state
                .finance()
                .get_account(destination)
                .expect("destination account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&state);
    }

    #[test]
    fn save_round_trip_preserves_budget_history_and_remaining_authority() {
        let (mut state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        let transaction = validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Persisted delegated allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-1_000),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(1_000),
                    },
                ],
                authorization: Some(authorization),
            },
        )
        .expect("budgeted transaction should validate")
        .commit(&mut state)
        .expect("budgeted transaction should commit");

        let registry = build_registry();
        let envelope = build_save(&registry, &state).expect("valid state should build a save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&registry, decoded).expect("current save should restore");

        let usage = resolve_budget_usage(&restored, mandate, restored.now())
            .expect("restored budget usage should resolve");
        assert_eq!(usage.used, Money::from_cents(1_000));
        assert_eq!(usage.remaining, Money::from_cents(1_500));
        assert_eq!(
            restored.finance().transactions_for_mandate(mandate).count(),
            1
        );
        let persisted_usage = restored
            .finance()
            .get_transaction(transaction)
            .expect("restored transaction should exist")
            .budget_usage()
            .expect("restored transaction should preserve its authority snapshot");
        assert_eq!(persisted_usage.mandate(), mandate);
        assert_eq!(persisted_usage.mandate_version(), 1);
        assert_eq!(persisted_usage.manager(), authorization.manager);
        assert_eq!(persisted_usage.scope(), authorization.scope);
        validate_invariants(&restored);
    }

    #[test]
    fn delegated_spend_rejects_manager_who_does_not_own_mandate() {
        let registry = build_registry();
        let (mut state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        let organization = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should exist")
            .organization();
        let other_manager = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Other Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("second manager fixture should validate");
        let invalid_authorization = MandateAuthority {
            manager: other_manager,
            ..authorization
        };

        let error = match validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Unauthorized delegated allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-500),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(500),
                    },
                ],
                authorization: Some(invalid_authorization),
            },
        ) {
            Ok(_) => panic!("foreign manager must not exercise another manager's mandate"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            FinanceError::Delegation(DelegationError::AuthorityManagerMismatch {
                mandate,
                manager: other_manager,
                expected: authorization.manager,
            })
        );
        assert_eq!(
            state
                .finance()
                .get_account(funding)
                .expect("funding account should exist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            state
                .finance()
                .get_account(destination)
                .expect("destination account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&state);
    }

    #[test]
    fn delegated_spend_rejects_scope_outside_mandate() {
        let (state, authorization, funding, destination) = make_test_budget();
        let mandate = authorization.mandate;
        let invalid_scope = ResponsibilityScope::Function(ResponsibilityFunction::Operations);
        let invalid_authorization = MandateAuthority {
            scope: invalid_scope,
            ..authorization
        };

        let error = match validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Out-of-scope delegated allocation".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: funding,
                        amount: Money::from_cents(-500),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: Money::from_cents(500),
                    },
                ],
                authorization: Some(invalid_authorization),
            },
        ) {
            Ok(_) => panic!("mandate must not authorize spending outside its scopes"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            FinanceError::Delegation(DelegationError::ScopeOutsideMandate {
                mandate,
                scope: invalid_scope,
            })
        );
        assert_eq!(
            state
                .finance()
                .get_account(funding)
                .expect("funding account should exist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            state
                .finance()
                .get_account(destination)
                .expect("destination account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&state);
    }
}
