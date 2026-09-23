//! Focused tests for the daily payroll pass: funding order, member wage accounts,
//! proportional shortfall consequences, and day-boundary cadence.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::time::{SimDuration, SimTime};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const DAY_MINUTES: u32 = 1_440;

struct PayrollFixture {
    state: AppState,
    organization: OrganizationId,
    boss: CharacterId,
    member: CharacterId,
    treasury: FinancialAccountId,
}

#[derive(Serialize)]
struct RelationshipRecordWire {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
    version: u32,
}

fn replace_serialized_relationship_version(
    envelope: SaveEnvelope,
    original: &crate::social::RelationshipRecord,
    version: u32,
) -> SaveEnvelope {
    let replacement = RelationshipRecordWire {
        from: original.from(),
        to: original.to(),
        dimensions: original.dimensions(),
        version,
    };
    let original_bytes =
        bincode::serialize(original).expect("relationship record should serialize");
    let replacement_bytes =
        bincode::serialize(&replacement).expect("replacement relationship should serialize");
    assert_eq!(
        original_bytes.len(),
        replacement_bytes.len(),
        "version-only relationship corruption must preserve wire size"
    );
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized target relationship must appear exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout relationship corruption must remain decodable")
}

fn make_test_payroll_fixture() -> PayrollFixture {
    let registry = build_registry();
    let mut state = AppState::new(7);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Payroll Test Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Test Boss".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("boss fixture should validate");
    let member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Test Soldier".to_owned(),
            organization: Some(organization),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("member fixture should validate");
    let treasury = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("treasury fixture should validate");
    PayrollFixture {
        state,
        organization,
        boss,
        member,
        treasury,
    }
}

/// Adds a one-member criminal organization with an empty payroll treasury.
fn add_single_member_payroll_organization(
    registry: &Registry,
    state: &mut AppState,
    name: &str,
) -> (OrganizationId, CharacterId, FinancialAccountId) {
    let organization = insert_organization(
        registry,
        state,
        OrganizationDraft {
            name: name.to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("payroll organization should validate");
    let boss = insert_character(
        state,
        CharacterDraft {
            name: format!("{name} Boss"),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("payroll boss should validate");
    let treasury = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("payroll treasury should validate");
    (organization, boss, treasury)
}

/// Seeds a treasury balance from an external counterparty so balances stay ledger-consistent.
fn credit_account(
    state: &mut AppState,
    payer: CharacterId,
    account: FinancialAccountId,
    cents: i64,
) {
    let counterparty = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Character(payer),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("counterparty account should validate");
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "test seed capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account,
                    amount: Money::from_cents(cents),
                },
                LedgerPosting {
                    account: counterparty,
                    amount: Money::from_cents(-cents),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transaction should validate")
    .commit(state)
    .expect("seed transaction should commit");
}

#[test]
fn payroll_is_due_only_on_nonzero_day_boundaries() {
    assert!(!is_payroll_due(SimTime::ZERO));
    assert!(!is_payroll_due(SimTime::from_minutes(1_439)));
    assert!(is_payroll_due(SimTime::from_minutes(1_440)));
    assert!(!is_payroll_due(SimTime::from_minutes(2_879)));
    assert!(is_payroll_due(SimTime::from_minutes(2_880)));
}

#[test]
fn payroll_account_selection_respects_terminal_version_capacity() {
    assert!(payroll_account_has_posting_headroom(1));
    assert!(payroll_account_has_posting_headroom(u32::MAX - 1));
    assert!(!payroll_account_has_posting_headroom(u32::MAX));
}

#[test]
fn funded_payroll_moves_wages_into_member_pockets() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let per_member = registry.upkeep().per_member_daily();
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100_000);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes =
        apply_daily_payroll(&registry, &mut fixture.state).expect("funded payroll should settle");

    assert_eq!(outcomes.len(), 1);
    let outcome = &outcomes[0];
    assert_eq!(outcome.organization(), fixture.organization);
    assert_eq!(outcome.short(), Money::ZERO);
    assert!(
        outcome.transaction().is_some(),
        "a funded payroll must expose its canonical ledger transaction"
    );
    assert_eq!(
        outcome.paid(),
        per_member
            .checked_mul(2)
            .expect("two-member payroll must fit money"),
        "boss and soldier are both paid members"
    );

    // The soldier's pocket was created and funded; the treasury paid both wages.
    let pocket = fixture
        .state
        .finance
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("paid member must hold a wage account");
    assert_eq!(pocket.balance(), per_member);
    let treasury_balance = fixture
        .state
        .finance
        .get_account(fixture.treasury)
        .expect("treasury must persist")
        .balance();
    assert_eq!(
        treasury_balance,
        Money::from_cents(100_000)
            .checked_sub(
                per_member
                    .checked_mul(2)
                    .expect("two-member payroll must fit money")
            )
            .expect("payroll cost must fit money")
    );
    // No shortfall means no resentment edge toward the supervisor was manufactured.
    assert!(
        fixture
            .state
            .social
            .get_relationship(fixture.member, fixture.boss)
            .is_none()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn daily_payroll_reserves_all_organization_ledger_ids_before_first_payment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let rival = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Second Payroll Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second criminal organization should validate");
    let rival_boss = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second Payroll Boss".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second payroll member should validate");
    let rival_treasury = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("second treasury should validate");
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100_000);
    credit_account(&mut fixture.state, rival_boss, rival_treasury, 100_000);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::LedgerTransaction, u32::MAX - 1);
    let before =
        bincode::serialize(&fixture.state).expect("pre-payroll allocator state should serialize");

    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("ledger-ID exhaustion is a terminal autonomous no-op");
    assert!(outcomes.is_empty());
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected payroll state should serialize"),
        before,
        "allocator exhaustion must reject the complete daily payroll before the first organization pays"
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("first treasury should persist")
            .balance(),
        Money::from_cents(100_000)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(rival_treasury)
            .expect("second treasury should persist")
            .balance(),
        Money::from_cents(100_000)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn daily_payroll_reserves_all_wage_account_ids_before_first_payment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let (_rival, rival_boss, rival_treasury) =
        add_single_member_payroll_organization(&registry, &mut fixture.state, "Account Rail Crew");
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100_000);
    credit_account(&mut fixture.state, rival_boss, rival_treasury, 100_000);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));

    // The two-member fixture plus the one-member rival need three new StreetCash wage accounts.
    // Leave capacity for only two. No organization may receive a prefix payroll.
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::FinancialAccount, u32::MAX - 2);
    let before =
        bincode::serialize(&fixture.state).expect("pre-payroll account rail should serialize");

    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("wage-account ID exhaustion is a terminal autonomous no-op");
    assert!(outcomes.is_empty());
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected payroll state should serialize"),
        before,
        "wage-account exhaustion must reject the complete daily payroll"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn daily_payroll_reserves_later_player_shortfall_report_before_rival_payment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let (player, _player_boss, _player_treasury) =
        add_single_member_payroll_organization(&registry, &mut fixture.state, "Later Player Crew");
    crate::world::world_system::designate_player_organization(&mut fixture.state, player)
        .expect("later criminal organization should be eligible as player organization");
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100_000);
    // The later player crew intentionally has no spendable cash, so it owes a shortfall report.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Report, u32::MAX);
    let before =
        bincode::serialize(&fixture.state).expect("pre-payroll report rail should serialize");

    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("shortfall-report ID exhaustion is a terminal autonomous no-op");
    assert!(outcomes.is_empty());
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected payroll state should serialize"),
        before,
        "later player report exhaustion must not let the earlier rival organization get paid"
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("earlier treasury should persist")
            .balance(),
        Money::from_cents(100_000)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn daily_payroll_skips_later_terminal_relationship_consequence_without_blocking_payments() {
    let registry = build_registry();
    let mut state = AppState::new(0xDA11_A701C);

    let (_first_org, first_boss, first_treasury) =
        add_single_member_payroll_organization(&registry, &mut state, "Earlier Funded Crew");
    credit_account(&mut state, first_boss, first_treasury, 100_000);

    let later_org = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Later Unpaid Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("later criminal organization should validate");
    let later_boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Later Unpaid Boss".to_owned(),
            organization: Some(later_org),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("later boss should validate");
    let later_member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Later Unpaid Member".to_owned(),
            organization: Some(later_org),
            supervisor: Some(later_boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("later member should validate");
    let mut dimensions = RelationshipDimensions::zero();
    dimensions.resentment =
        RelationshipLevel::try_new(1).expect("fixture resentment should be valid");
    validate_set_relationship(&state, later_member, later_boss, dimensions)
        .expect("fixture relationship should validate")
        .commit(&mut state)
        .expect("fixture relationship should commit");

    let original = state
        .social()
        .get_relationship(later_member, later_boss)
        .expect("fixture relationship should persist")
        .clone();
    let envelope = build_save(&registry, &state).expect("payroll fixture should save");
    let corrupted = replace_serialized_relationship_version(envelope, &original, u32::MAX);
    state = restore_save(&registry, corrupted)
        .expect("max-version relationship remains structurally valid");
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut state)
        .expect("terminal relationship capacity must not block mandatory payroll");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        state
            .finance()
            .get_account(first_treasury)
            .expect("earlier treasury should persist")
            .balance(),
        Money::from_cents(100_000 - registry.upkeep().per_member_daily().cents()),
        "the earlier funded organization must still complete payroll"
    );
    let exhausted = state
        .social()
        .get_relationship(later_member, later_boss)
        .expect("terminal relationship should persist");
    assert_eq!(exhausted.version(), u32::MAX);
    assert_eq!(
        exhausted.dimensions(),
        original.dimensions(),
        "an exhausted relationship cannot represent additional shortfall resentment"
    );
    let later = outcomes
        .iter()
        .find(|outcome| outcome.organization() == later_org)
        .expect("later unfunded organization must still resolve payroll");
    assert_eq!(later.paid(), Money::ZERO);
    assert_eq!(later.short(), later.owed());
    validate_state(&state)
        .expect("terminal-relationship payroll state should remain release-valid");
    validate_invariants(&state);
}

#[test]
fn detention_does_not_erase_standing_daily_wage_obligation() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Payroll Custody Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: police,
            title: "Payroll custody test".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.member)]),
        },
    )
    .expect("payroll custody investigation should validate")
    .commit(&mut fixture.state)
    .expect("payroll custody investigation should commit");
    let mut evidence = BTreeSet::new();
    for kind in [EvidenceKind::Document, EvidenceKind::Fingerprint] {
        let id = validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(fixture.member),
                origin: None,
                kind,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("payroll custody evidence should validate")
        .commit(&mut fixture.state)
        .expect("payroll custody evidence should commit");
        evidence.insert(id);
    }
    validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.member,
            investigation,
            evidence,
        },
    )
    .expect("payroll member detention should validate")
    .commit(&mut fixture.state)
    .expect("payroll member detention should commit");
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.member)
            .is_some(),
        "fixture member must still be detained when payroll runs"
    );

    let per_member = registry.upkeep().per_member_daily();
    let owed = per_member.checked_mul(2).expect("two wages must fit money");
    credit_account(
        &mut fixture.state,
        fixture.boss,
        fixture.treasury,
        owed.cents(),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.member)
            .is_some(),
        "fixture member must still be detained at the payroll boundary"
    );

    let outcome = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("detention must not invalidate standing payroll")
        .into_iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("staffed criminal organization must run payroll");
    assert_eq!(outcome.owed(), owed);
    assert_eq!(outcome.paid(), owed);
    assert_eq!(outcome.short(), Money::ZERO);
    let member_pocket = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("detained rostered member still receives the standing daily wage");
    assert_eq!(member_pocket.balance(), per_member);
    validate_invariants(&fixture.state);
}

#[test]
fn accounted_funds_are_available_for_payroll_but_settlement_balances_are_not() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let accounted = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted payroll reserve should validate");
    let settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement fixture should validate");
    let owed = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(2)
        .expect("two-member payroll should fit money");
    credit_account(&mut fixture.state, fixture.boss, accounted, owed.cents());
    credit_account(
        &mut fixture.state,
        fixture.boss,
        settlement,
        owed.cents() * 10,
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcome = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("accounted funds should fund ordinary wages")
        .into_iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("staffed organization should run payroll");

    assert_eq!(outcome.paid(), owed);
    assert_eq!(outcome.short(), Money::ZERO);
    let transaction = fixture
        .state
        .finance()
        .get_transaction(
            outcome
                .transaction()
                .expect("funded payroll must expose its ledger transaction"),
        )
        .expect("payroll ledger transaction should persist");
    assert!(transaction.postings().iter().any(|posting| {
        posting.account == accounted && posting.amount == owed.checked_neg().expect("owed negates")
    }));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(accounted)
            .expect("accounted reserve should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(settlement)
            .expect("settlement account should persist")
            .balance(),
        Money::from_cents(owed.cents() * 10),
        "settlement balances are ledger counterparties, not spendable payroll liquidity"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn payroll_spends_dirty_cash_before_clean_reserves() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let concealed = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("concealed payroll reserve should validate");
    let accounted = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted payroll reserve should validate");
    let owed = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(2)
        .expect("two-member payroll should fit money");
    assert!(
        owed.cents() > 300,
        "fixture payroll must exercise every funding tier"
    );

    // Make the clean account by far the largest balance. Balance-first funding would consume it
    // immediately; semantic funding should exhaust easy street cash, then concealed reserves,
    // and use accounted money only for the remainder.
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100);
    credit_account(&mut fixture.state, fixture.boss, concealed, 200);
    credit_account(&mut fixture.state, fixture.boss, accounted, owed.cents());

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcome = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("mixed-source payroll should settle")
        .into_iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("staffed organization should run payroll");

    assert_eq!(outcome.paid(), owed);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("street treasury should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(concealed)
            .expect("concealed reserve should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(accounted)
            .expect("accounted reserve should persist")
            .balance(),
        Money::from_cents(300),
        "clean capital should cover only the amount dirty cash could not fund"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn shortfall_distributes_available_cash_and_breeds_supervisor_resentment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    // Even a tiny treasury is distributed across the active crew.
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 10);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("short payroll should settle proportionally");

    let outcome = outcomes
        .iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("a staffed criminal organization must run payroll");
    assert_eq!(outcome.paid(), Money::from_cents(10));
    assert_eq!(
        outcome.short(),
        outcome
            .owed()
            .checked_sub(Money::from_cents(10))
            .expect("partial payment cannot exceed payroll owed")
    );

    // The shorted member resents their supervisor through the canonical relationship path.
    let relationship = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions())
        .expect("shortfall must create a resentment edge toward the supervisor");
    assert_eq!(
        relationship.resentment.value(),
        registry.upkeep().shortfall_resentment(),
        "fresh resentment edge carries exactly the authored increment"
    );

    // The available ten cents are split evenly across the two current organization members.
    let pocket = fixture
        .state
        .finance
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("a partially paid member receives a wage account");
    assert_eq!(pocket.balance(), Money::from_cents(5));
    validate_invariants(&fixture.state);
}

#[test]
fn one_cent_shortfall_rotates_rounding_priority_across_payroll_days() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let per_member = registry.upkeep().per_member_daily();
    let owed = per_member.checked_mul(2).expect("two wages must fit money");
    credit_account(
        &mut fixture.state,
        fixture.boss,
        fixture.treasury,
        owed.cents() - 1,
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcome = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("one-cent-short payroll should settle")
        .into_iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("staffed organization must run payroll");

    assert_eq!(outcome.paid(), Money::from_cents(owed.cents() - 1));
    assert_eq!(outcome.short(), Money::from_cents(1));
    let boss_pocket = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.boss))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("boss receives a wage account");
    let member_pocket = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("member receives partial wage");
    assert_eq!(boss_pocket.balance(), per_member);
    assert_eq!(
        member_pocket.balance(),
        Money::from_cents(per_member.cents() - 1)
    );
    assert!(
        fixture
            .state
            .social()
            .get_relationship(fixture.boss, fixture.boss)
            .is_none(),
        "fully paid member does not receive a shortfall consequence"
    );
    assert_eq!(
        fixture
            .state
            .social()
            .get_relationship(fixture.member, fixture.boss)
            .expect("the one-cent-shorted member resents their supervisor")
            .dimensions()
            .resentment
            .value(),
        1,
        "a one-cent rounding shortfall should cause only minimal resentment"
    );

    // Fund the same one-cent-short payroll on day two. The extra cent rotates to the soldier,
    // so the boss takes this day's one-cent shortfall instead of CharacterId order making the
    // subordinate the permanent rounding loser.
    credit_account(
        &mut fixture.state,
        fixture.boss,
        fixture.treasury,
        owed.cents() - 1,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    apply_daily_payroll(&registry, &mut fixture.state)
        .expect("second one-cent-short payroll should settle");

    let expected_two_day_total = Money::from_cents(
        per_member
            .cents()
            .checked_mul(2)
            .and_then(|cents| cents.checked_sub(1))
            .expect("two-day wage total should fit"),
    );
    let boss_balance = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.boss))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("boss wage account persists")
        .balance();
    let member_balance = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("member wage account persists")
        .balance();
    assert_eq!(boss_balance, expected_two_day_total);
    assert_eq!(member_balance, expected_two_day_total);
    assert_eq!(
        fixture
            .state
            .social()
            .get_relationship(fixture.member, fixture.boss)
            .expect("day-one rounding shortfall remains represented")
            .dimensions()
            .resentment
            .value(),
        1,
        "the subordinate must not gain another resentment point when day-two rounding favors them"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn three_member_rounding_rotation_advances_in_stable_member_order() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let third = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Third Payroll Member".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(fixture.boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("third payroll member should validate");
    assert!(
        fixture.boss < fixture.member && fixture.member < third,
        "fixture creation order must match the stable member-ID order"
    );

    let per_member = registry.upkeep().per_member_daily();
    let owed = per_member
        .checked_mul(3)
        .expect("three wages must fit money");
    for _ in 0..2 {
        credit_account(
            &mut fixture.state,
            fixture.boss,
            fixture.treasury,
            owed.cents() - 1,
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
        apply_daily_payroll(&registry, &mut fixture.state)
            .expect("one-cent-short three-member payroll should settle");
    }

    let balance = |character| {
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Character(character))
            .find(|account| account.kind() == AccountKind::StreetCash)
            .expect("each member should retain a wage account")
            .balance()
    };
    let two_full_wages = per_member
        .checked_mul(2)
        .expect("two full wages must fit money");
    let one_cent_short = Money::from_cents(two_full_wages.cents() - 1);

    // Day one awards the two remainder cents to the first two stable members. Day two advances
    // that two-member remainder window by one slot, so the middle member is paid in full twice.
    // With three members, advancing backward instead of forward is observably different.
    assert_eq!(balance(fixture.boss), one_cent_short);
    assert_eq!(balance(fixture.member), two_full_wages);
    assert_eq!(balance(third), one_cent_short);
    validate_invariants(&fixture.state);
}

#[test]
fn half_paid_wage_causes_half_of_full_shortfall_resentment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let per_member = registry.upkeep().per_member_daily();
    // Two current organization members split one wage evenly, leaving each exactly half paid. The boss has
    // no supervisor, while the subordinate's relationship consequence should reflect the
    // severity of their own shortage rather than treating all underpayment as nonpayment.
    credit_account(
        &mut fixture.state,
        fixture.boss,
        fixture.treasury,
        per_member.cents(),
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    apply_daily_payroll(&registry, &mut fixture.state)
        .expect("half-funded payroll should settle proportionally");

    let resentment = fixture
        .state
        .social()
        .get_relationship(fixture.member, fixture.boss)
        .expect("half-paid subordinate should resent their supervisor")
        .dimensions()
        .resentment
        .value();
    assert_eq!(
        resentment,
        registry.upkeep().shortfall_resentment().div_ceil(2),
        "relationship damage should scale with the uncovered half of the wage"
    );
    validate_invariants(&fixture.state);
}

/// A chronically insolvent organization must clamp crew resentment at the authored rail
/// instead of panicking once accumulated resentment leaves the bounded 0..=100 range.
#[test]
fn repeated_shortfalls_clamp_resentment_at_the_authored_rail() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let increment = registry.upkeep().shortfall_resentment();

    // Enough consecutive unpaid days that fresh resentment crosses the authored rail;
    // without the saturating raise the crossing day panicked on the bounded range.
    let days_to_cross_rail =
        u32::from(crate::social::RelationshipLevel::MAX_VALUE).div_ceil(u32::from(increment));
    for _ in 0..days_to_cross_rail {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
        apply_daily_payroll(&registry, &mut fixture.state)
            .expect("repeated shortfall payroll should settle");
    }
    let at_rail = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions().resentment.value())
        .expect("repeated shortfalls must create a resentment edge");
    assert_eq!(at_rail, crate::social::RelationshipLevel::MAX_VALUE);

    // The day that crosses the rail clamps instead of panicking, and stays clamped after.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    apply_daily_payroll(&registry, &mut fixture.state)
        .expect("clamped shortfall payroll should settle");
    let clamped = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .expect("clamped edge must persist")
        .dimensions()
        .resentment
        .value();
    assert_eq!(clamped, crate::social::RelationshipLevel::MAX_VALUE);
    validate_invariants(&fixture.state);
}

/// An organization holding no liquid cash accounts at all is fully short on payroll,
/// exactly like one whose accounts hold too little.
#[test]
fn organization_without_any_funding_accounts_still_incurs_full_shortfall() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("unfunded payroll should still record the shortfall");

    let outcome = outcomes
        .iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("a staffed criminal organization with no cash still owes wages");
    assert_eq!(outcome.paid(), Money::ZERO);
    assert_eq!(outcome.short(), outcome.owed());
    let relationship = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions())
        .expect("unpaid crew resents the supervisor even with no accounts");
    assert_eq!(
        relationship.resentment.value(),
        registry.upkeep().shortfall_resentment()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn shortfall_reports_once_to_the_player_organization_only() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .expect("criminal organization should be eligible as the player organization");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));

    let outcomes = apply_daily_payroll(&registry, &mut fixture.state)
        .expect("player shortfall payroll should settle and report");
    assert!(!outcomes.is_empty());

    let payroll_reports: Vec<_> = fixture
        .state
        .reports()
        .reports_for(fixture.organization)
        .filter(|report| report.title() == "Payroll ran short")
        .collect();
    assert_eq!(
        payroll_reports.len(),
        1,
        "one shortfall produces exactly one notable report"
    );
    let entry = &payroll_reports[0].entries()[0];
    assert!(entry.summary.contains("went uncovered"));
    validate_invariants(&fixture.state);
}

#[test]
fn player_shortfall_report_exhaustion_rejects_before_money_or_resentment_moves() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 10);
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .expect("criminal organization should be eligible as the player organization");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let treasury_before = fixture
        .state
        .finance()
        .get_account(fixture.treasury)
        .expect("treasury should persist")
        .balance();
    let account_next_before = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let transaction_next_before = fixture.state.ids.next_raw(IdKind::LedgerTransaction);
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Report, u32::MAX);

    let plan = plan_organization_payroll(&registry, &fixture.state, fixture.organization)
        .expect("player payroll should plan")
        .expect("fixture organization has members");
    let error = apply_organization_payroll(&registry, &mut fixture.state, plan)
        .expect_err("report exhaustion must reject before payroll mutation");
    assert!(matches!(
        error,
        PayrollError::IdExhaustion(IdExhaustionError::Exhausted { kind: "report", .. })
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("treasury should persist")
            .balance(),
        treasury_before
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        account_next_before
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::LedgerTransaction),
        transaction_next_before
    );
    assert!(
        fixture
            .state
            .social()
            .get_relationship(fixture.member, fixture.boss)
            .is_none()
    );
    assert!(
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Character(fixture.member))
            .all(|account| account.kind() != AccountKind::StreetCash)
    );
}

#[test]
fn player_shortfall_ledger_exhaustion_rejects_before_money_accounts_resentment_or_report_move() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 10);
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .expect("criminal organization should be eligible as the player organization");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let treasury_before = fixture
        .state
        .finance()
        .get_account(fixture.treasury)
        .expect("treasury should persist")
        .balance();
    let account_next_before = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let report_next_before = fixture.state.ids.next_raw(IdKind::Report);
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::LedgerTransaction, u32::MAX);

    let plan = plan_organization_payroll(&registry, &fixture.state, fixture.organization)
        .expect("player payroll should plan")
        .expect("fixture organization has members");
    let error = apply_organization_payroll(&registry, &mut fixture.state, plan)
        .expect_err("ledger exhaustion must reject the whole shortfall composite");
    assert!(matches!(
        error,
        PayrollError::Finance(FinanceError::IdExhaustion(IdExhaustionError::Exhausted {
            kind: "ledger transaction",
            ..
        }))
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("treasury should persist")
            .balance(),
        treasury_before
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        account_next_before,
        "failed payroll must not consume planned wage-account IDs"
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::Report),
        report_next_before,
        "report capacity preflight is read-only and must not consume its ID"
    );
    assert!(
        fixture
            .state
            .social()
            .get_relationship(fixture.member, fixture.boss)
            .is_none(),
        "failed payroll must not publish shortfall resentment"
    );
    assert!(
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Character(fixture.member))
            .all(|account| account.kind() != AccountKind::StreetCash),
        "failed payroll must not leave an empty planned wage account"
    );
    assert_eq!(
        fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .filter(|report| report.title() == "Payroll ran short")
            .count(),
        0,
        "failed payroll must not publish the shortfall report"
    );
    validate_invariants(&fixture.state);
}
