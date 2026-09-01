//! Focused tests for account management, transfers, and ledger balance invariants.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::delegation::delegation_system::{
    DelegationError, MandateRevisionDraft, validate_assign_mandate, validate_revise_mandate,
};
use crate::delegation::{
    BudgetAuthority, BudgetPeriod, MandateAuthority, MandateDraft, ResponsibilityFunction,
    ResponsibilityScope,
};
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
};
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
    validate_transfer_business_ownership,
};
use crate::world::{
    AutonomyLevel, BusinessOwner, CharacterDraft, OrganizationDraft, OrganizationKind,
};
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
fn planned_account_transaction_opens_and_funds_account_atomically() {
    let registry = build_registry();
    let mut state = AppState::new(32);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Atomic Account Test".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let owner = FinancialOwner::Organization(organization);
    let source = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .expect("source account should validate");
    let openings = validate_open_accounts(
        &state,
        vec![FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        }],
    )
    .expect("planned account should validate");
    let planned = openings
        .account_id(0)
        .expect("one planned account must expose one id");

    validate_record_transaction_with_openings(
        &state,
        openings,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Open funded street pocket".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-500),
                },
                LedgerPosting {
                    account: planned,
                    amount: Money::from_cents(500),
                },
            ],
            authorization: None,
        },
    )
    .expect("transaction over planned account should validate")
    .commit(&mut state)
    .expect("planned opening and transaction should commit together");

    let account = state
        .finance()
        .get_account(planned)
        .expect("planned account should be opened by the transaction");
    assert_eq!(account.owner(), owner);
    assert_eq!(account.kind(), AccountKind::StreetCash);
    assert_eq!(account.balance(), Money::from_cents(500));
    validate_invariants(&state);
}

#[test]
fn rejected_transaction_over_planned_account_consumes_no_account_id() {
    let registry = build_registry();
    let mut state = AppState::new(33);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Rejected Opening Test".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let owner = FinancialOwner::Organization(organization);
    let before_next = state.ids.next_raw(IdKind::FinancialAccount);
    let openings = validate_open_accounts(
        &state,
        vec![FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        }],
    )
    .expect("planned account should validate");
    let planned = openings
        .account_id(0)
        .expect("one planned account must expose one id");

    let error = match validate_record_transaction_with_openings(
        &state,
        openings,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Invalid planned opening".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: planned,
                    amount: Money::from_cents(500),
                },
                LedgerPosting {
                    account: planned,
                    amount: Money::from_cents(-500),
                },
            ],
            authorization: None,
        },
    ) {
        Ok(_) => panic!("duplicate-account transaction must reject before opening anything"),
        Err(error) => error,
    };
    assert_eq!(error, FinanceError::DuplicateAccount(planned));
    assert_eq!(state.ids.next_raw(IdKind::FinancialAccount), before_next);
    assert!(state.finance().get_account(planned).is_none());
    validate_invariants(&state);
}

#[test]
fn validated_budget_transaction_remains_valid_when_hierarchy_change_is_blocked() {
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
    validate_revise_mandate(&state, mandate, revision)
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
    let (mut state, authorization, funding, destination) = make_test_budget();
    let mandate = authorization.mandate;
    let organization = state
        .delegation()
        .get_mandate(mandate)
        .expect("mandate should exist")
        .organization();
    let other_manager = insert_character(
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

struct LaunderingFixture {
    state: AppState,
    organization: crate::core::id::OrganizationId,
    street: FinancialAccountId,
    accounted: FinancialAccountId,
    business: crate::core::id::BusinessId,
}

fn make_laundering_fixture() -> LaunderingFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xC0FFEE);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Laundering Test Organization".to_owned(),
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
    .expect("street account should validate");
    let accounted = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted account should validate");
    let neighborhood = crate::world::world_system::insert_neighborhood(
        &mut state,
        crate::world::NeighborhoodDraft {
            name: "Laundering Ward".to_owned(),
            profile: crate::world::NeighborhoodProfile {
                economy: crate::world::NeighborhoodEconomyProfile {
                    wealth: crate::world::Rating::try_new(60)
                        .expect("fixture rating should validate"),
                    commercial_activity: crate::world::Rating::try_new(70)
                        .expect("fixture rating should validate"),
                    illicit_demand: crate::world::Rating::try_new(30)
                        .expect("fixture rating should validate"),
                },
                institutions: crate::world::NeighborhoodInstitutionProfile {
                    police_presence: crate::world::Rating::try_new(40)
                        .expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = crate::world::world_system::insert_business(
        &registry,
        &mut state,
        crate::world::BusinessDraft {
            name: "Clean Laundromat".to_owned(),
            kind: crate::world::BusinessKind::Retail,
            functions: BTreeSet::from([crate::world::BusinessFunction::CashIntensive]),
            neighborhood,
            owner: crate::world::BusinessOwner::Organization(organization),
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
    crate::economy::business_economy_system::validate_establish_business_economy(
        &registry,
        &state,
        crate::economy::BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("business economy fixture should validate")
    .commit(&mut state)
    .expect("business economy fixture should commit");
    LaunderingFixture {
        state,
        organization,
        street,
        accounted,
        business,
    }
}

#[test]
fn laundering_moves_street_cash_to_accounted_funds_minus_the_authored_fee() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();

    // Seed the street account from an off-book concealed reserve through a balanced transfer.
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Move take to street".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");

    // Capacity is the authored fraction of the front's legitimate gross potential; stay inside it.
    let gross_potential =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("gross potential should resolve");
    let capacity = Money::from_cents(
        (i128::from(gross_potential.cents())
            * i128::from(registry.laundering().plausibility_gross_basis_points())
            / 10_000)
            .try_into()
            .expect("capacity should fit money"),
    );
    assert!(
        capacity.cents() < 1_000_000,
        "fixture expects a capacity below the seeded cash"
    );

    let validated = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: capacity,
        },
    )
    .expect("in-capacity laundering should validate")
    .commit(&mut fixture.state)
    .expect("in-capacity laundering should commit");

    let fee = Money::from_cents(
        (i128::from(capacity.cents()) * i128::from(registry.laundering().fee_basis_points())
            / 10_000)
            .try_into()
            .expect("fee should fit money"),
    );
    let transaction = fixture
        .state
        .finance()
        .get_transaction(validated)
        .expect("laundering transaction should persist");
    let street_delta: i64 = transaction
        .postings()
        .iter()
        .filter(|posting| posting.account == fixture.street)
        .map(|posting| posting.amount.cents())
        .sum();
    let accounted_delta: i64 = transaction
        .postings()
        .iter()
        .filter(|posting| posting.account == fixture.accounted)
        .map(|posting| posting.amount.cents())
        .sum();
    assert_eq!(street_delta, -capacity.cents());
    assert_eq!(accounted_delta, capacity.cents() - fee.cents());
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_token_rejects_after_front_changes_owner() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    let rival = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Rival Front Buyer".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival organization should validate");
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund stale laundering test".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-100),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(100),
                },
            ],
            authorization: None,
        },
    )
    .expect("fixture funding should validate")
    .commit(&mut fixture.state)
    .expect("fixture funding should commit");
    let token = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(100),
        },
    )
    .expect("laundering should validate while the front is owned");
    let street_before = fixture
        .state
        .finance()
        .get_account(fixture.street)
        .expect("street account should persist")
        .balance();
    let accounted_before = fixture
        .state
        .finance()
        .get_account(fixture.accounted)
        .expect("accounted account should persist")
        .balance();

    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(rival),
    )
    .expect("front ownership transfer should validate")
    .commit(&mut fixture.state)
    .expect("front ownership transfer should commit");
    let error = token
        .commit(&mut fixture.state)
        .expect_err("a laundering token cannot survive loss of the front");
    assert!(matches!(
        error,
        LaunderingError::StaleBusiness {
            business,
            ..
        } if business == fixture.business
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.street)
            .expect("street account should persist")
            .balance(),
        street_before
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("accounted account should persist")
            .balance(),
        accounted_before
    );
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_above_plausible_capacity_is_rejected_without_state_change() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    // Seed more street cash than any plausible capacity so the rejection proves the
    // plausibility ceiling, not an empty source (the empty-source case has its own test).
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "seed street cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");
    let gross_potential =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("gross potential should resolve");
    let capacity = Money::from_cents(
        (i128::from(gross_potential.cents())
            * i128::from(registry.laundering().plausibility_gross_basis_points())
            / 10_000)
            .try_into()
            .expect("capacity should fit money"),
    );
    let before_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(capacity.cents() + 1),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("over-capacity laundering must be rejected"),
    };
    assert!(matches!(error, LaunderingError::CapacityExceeded { .. }));
    let after_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    assert_eq!(before_balances, after_balances);
}

#[test]
fn laundering_requires_a_street_cash_source_account() {
    let registry = build_registry();
    let fixture = make_laundering_fixture();

    // An AccountedFunds source fails the street-cash kind check.
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.accounted, // wrong kind: AccountedFunds as source
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(1_000),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("non-street-cash source must be rejected"),
    };
    assert!(matches!(
        error,
        LaunderingError::InvalidStreetAccountKind(_)
    ));
}

/// A source that does not actually hold the requested cash must be rejected: debiting a
/// phantom balance would mint accounted funds out of nothing.
#[test]
fn laundering_more_than_the_street_balance_is_rejected_without_minting_funds() {
    let registry = build_registry();
    let fixture = make_laundering_fixture();
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.street)
            .expect("street account")
            .balance(),
        Money::ZERO,
        "fixture leaves the street account empty"
    );

    let before_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(1_000),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("laundering beyond the street balance must be rejected"),
    };
    let LaunderingError::InsufficientStreetCash {
        account,
        balance_cents,
        requested_cents,
    } = error
    else {
        panic!("expected InsufficientStreetCash, found {error}");
    };
    assert_eq!(account, fixture.street);
    assert_eq!(balance_cents, 0);
    assert_eq!(requested_cents, 1_000);
    let after_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    assert_eq!(before_balances, after_balances);
}

/// Plausibility tracks the front's current earning power: a sabotage-disrupted front earns
/// the authored degraded fraction, so its plausible laundering capacity shrinks with it.
#[test]
fn disrupted_front_capacity_shrinks_with_degraded_books() {
    use crate::economy::business_economy_system::{
        resolve_business_current_gross, validate_disrupt_business_economy,
    };
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    // Seed ample street cash up front so capacity rejections prove the plausibility ceiling.
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "seed street cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("disruption should validate")
        .commit(&mut fixture.state)
        .expect("disruption should commit");

    let current_gross = resolve_business_current_gross(&registry, &fixture.state, fixture.business)
        .expect("disrupted gross should resolve");
    let degraded_capacity = crate::finance::helpers::resolve_basis_point_share(
        current_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("degraded capacity should fit money");
    // The disruption must actually bite: degraded books support strictly less volume than
    // the front's healthy gross potential would.
    let healthy_gross =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("healthy gross should resolve");
    let healthy_capacity = crate::finance::helpers::resolve_basis_point_share(
        healthy_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("healthy capacity should fit money");
    assert!(
        degraded_capacity.cents() > 0 && degraded_capacity < healthy_capacity,
        "fixture expects a nontrivial degraded capacity below the healthy one"
    );

    // One cent beyond the degraded capacity is rejected even though the same volume would
    // have fit the front's healthy books.
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(degraded_capacity.cents() + 1),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("over-degraded-capacity laundering must be rejected"),
    };
    assert!(matches!(error, LaunderingError::CapacityExceeded { .. }));

    // And exactly the degraded capacity still launders through the canonical path.
    validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: degraded_capacity,
        },
    )
    .expect("exactly-degraded-capacity laundering should validate")
    .commit(&mut fixture.state)
    .expect("exactly-degraded-capacity laundering should commit");
    validate_invariants(&fixture.state);
}
