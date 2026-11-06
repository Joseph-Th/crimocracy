//! Budget projection regression tests through canonical ledger and mandate commands.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{build_save, restore_save};
use crate::delegation::delegation_system::{
    MandateRevisionDraft, validate_assign_mandate, validate_revise_mandate,
};
use crate::delegation::{
    BudgetAuthority, BudgetPeriod, MandateAuthority, MandateDraft, ResponsibilityFunction,
    ResponsibilityScope,
};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

#[test]
fn lowering_budget_below_prior_spending_leaves_zero_allowance_without_forgiving_spending() {
    let registry = build_registry();
    let mut state = AppState::new(91);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Budget restriction fixture".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .unwrap();
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Budget manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let owner = FinancialOwner::Organization(organization);
    let funding = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let recipient = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let capital = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Fund delegated spending".into(),
            postings: vec![
                LedgerPosting {
                    account: capital,
                    amount: Money::from_cents(-2_000),
                },
                LedgerPosting {
                    account: funding,
                    amount: Money::from_cents(2_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
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
                limit: Money::from_cents(2_000),
                period: BudgetPeriod::Weekly,
            }),
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let authorization = MandateAuthority {
        mandate,
        manager,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Finance),
    };
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Delegated expenditure".into(),
            postings: vec![
                LedgerPosting {
                    account: funding,
                    amount: Money::from_cents(-1_200),
                },
                LedgerPosting {
                    account: recipient,
                    amount: Money::from_cents(1_200),
                },
            ],
            authorization: Some(authorization),
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let previous = state.delegation().get_mandate(mandate).unwrap();
    validate_revise_mandate(
        &state,
        mandate,
        MandateRevisionDraft {
            scopes: previous.scopes().clone(),
            standing_orders: previous.standing_orders().clone(),
            budget: Some(BudgetAuthority {
                funding_account: funding,
                limit: Money::from_cents(500),
                period: BudgetPeriod::Weekly,
            }),
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();

    let allowance = resolve_budget_usage(&state, mandate, state.now()).unwrap();
    assert_eq!(allowance.limit, Money::from_cents(500));
    assert_eq!(allowance.used, Money::from_cents(1_200));
    assert_eq!(allowance.remaining, Money::ZERO);

    let spending_after_restriction = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: "Denied after tightened limit".into(),
        postings: vec![
            LedgerPosting {
                account: funding,
                amount: Money::from_cents(-1),
            },
            LedgerPosting {
                account: recipient,
                amount: Money::from_cents(1),
            },
        ],
        authorization: Some(authorization),
    };
    let error = validate_record_transaction(&state, spending_after_restriction)
        .err()
        .unwrap();
    assert_eq!(
        error,
        FinanceError::BudgetExceeded {
            mandate,
            limit_cents: 500,
            used_cents: 1_200,
            requested_cents: 1,
        }
    );
    assert_eq!(
        state.finance().get_account(funding).unwrap().balance(),
        Money::from_cents(800)
    );
    validate_invariants(&state);

    // Budget use is reconstructed from persisted ledger postings, not cleared by a save/load
    // boundary. A loaded campaign must not grant a fresh allowance after the tighter policy.
    let saved = build_save(&registry, &state).expect("restricted-budget state should save");
    let restored = restore_save(&registry, saved).expect("budget indexes must reconstruct");
    let restored_usage = resolve_budget_usage(&restored, mandate, restored.now()).unwrap();
    assert_eq!(restored_usage, allowance);
    validate_invariants(&restored);
}
