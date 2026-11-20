//! Court judgments, actual counsel counterplay, fine collection and deterministic recovery.
use super::*;
use crate::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
use crate::core::id::FinancialAccountId;
use crate::core::simulation::run_test_tick;
use crate::core::time::{DAY_DURATION, SimDuration};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::legal::legal_representation_system::validate_retain_legal_representation;
use crate::legal::{LegalRepresentationDraft, LegalRepresentationOrigin};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};

#[test]
fn later_review_cannot_impose_second_fine_from_already_adjudicated_evidence() {
    let mut fixture = fixture();
    let first = daily_boundary(&fixture.registry, &mut fixture.state).opened_prosecution_cases[0];
    validate_release_arrest(&fixture.state, fixture.arrest)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    fixture.state.advance_clock(SimDuration::from_minutes(
        14 * DAY_DURATION.as_minutes() - 1,
    ));
    let tick = run_test_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.adjudicated_prosecution_cases, vec![first]);
    let initial = fixture.state.legal().get_prosecution_case(first).unwrap();
    assert_eq!(initial.status(), ProsecutionCaseStatus::Convicted);
    let evidence = initial.evidence().clone();
    let old_fine = initial.judgment().unwrap().fine();
    use crate::legal::investigation_system::{
        InvestigationTransition, validate_transition_investigation,
    };
    if fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .unwrap()
        .status()
        == crate::legal::InvestigationStatus::Active
    {
        validate_transition_investigation(
            &fixture.state,
            fixture.investigation,
            InvestigationTransition::Suspend,
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    }
    let second = validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    fixture.state.advance_clock(SimDuration::from_minutes(
        14 * DAY_DURATION.as_minutes() - 1,
    ));
    let tick = run_test_tick(&fixture.registry, &mut fixture.state);
    assert!(tick.adjudicated_prosecution_cases.is_empty());
    assert_eq!(tick.declined_prosecution_cases, vec![second]);
    let later = fixture.state.legal().get_prosecution_case(second).unwrap();
    assert_eq!(later.status(), ProsecutionCaseStatus::Declined);
    assert!(later.judgment().is_none());
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(first)
            .unwrap()
            .judgment()
            .unwrap()
            .fine(),
        old_fine
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

fn daily_boundary(
    registry: &Registry,
    state: &mut AppState,
) -> crate::core::simulation::TickOutcome {
    let delta = DAY_DURATION.as_minutes()
        - u32::try_from(state.now().as_minutes() % u64::from(DAY_DURATION.as_minutes())).unwrap();
    state.advance_clock(SimDuration::from_minutes(delta - 1));
    run_test_tick(registry, state)
}

fn account(state: &mut AppState, owner: FinancialOwner, kind: AccountKind) -> FinancialAccountId {
    insert_account(state, FinancialAccountDraft { owner, kind }).unwrap()
}

fn fund(state: &mut AppState, from: FinancialAccountId, to: FinancialAccountId, cents: i64) {
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Fixture funds".into(),
            authorization: None,
            postings: vec![
                LedgerPosting {
                    account: from,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: to,
                    amount: Money::from_cents(cents),
                },
            ],
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
}

fn sponsor(fixture: &Fixture) -> OrganizationId {
    fixture
        .state
        .world()
        .get_character(fixture.defendant)
        .unwrap()
        .organization()
        .unwrap()
}

fn retain_defense(fixture: &mut Fixture) -> crate::core::id::LegalRepresentationId {
    let sponsor = sponsor(fixture);
    let handler = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Defense Liaison".into(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let firm = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Legal Office".into(),
            kind: OrganizationKind::LegalServices,
        },
    )
    .unwrap();
    let counsel = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Defense Counsel".into(),
            organization: Some(firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(88))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let level = |n| RelationshipLevel::try_new(n).unwrap();
    validate_set_relationship(
        &fixture.state,
        handler,
        counsel,
        RelationshipDimensions {
            trust: level(70),
            respect: level(65),
            fear: level(0),
            affection: level(20),
            dependence: level(30),
            resentment: level(0),
            debt: level(15),
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let contact = validate_establish_contact(
        &fixture.state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: counsel,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let payer = account(
        &mut fixture.state,
        FinancialOwner::Organization(sponsor),
        AccountKind::StreetCash,
    );
    let source = account(
        &mut fixture.state,
        FinancialOwner::Organization(sponsor),
        AccountKind::Settlement,
    );
    let provider = account(
        &mut fixture.state,
        FinancialOwner::Organization(firm),
        AccountKind::LegitimateOperating,
    );
    fund(&mut fixture.state, source, payer, 5000);
    validate_retain_legal_representation(
        &fixture.state,
        LegalRepresentationDraft {
            arrest: fixture.arrest,
            sponsor,
            contact,
            fee: Money::from_cents(5000),
            payer_accounts: BTreeSet::from([payer]),
            provider_account: provider,
            authorization: None,
            origin: LegalRepresentationOrigin::DirectRetention,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap()
}

#[test]
fn court_hearing_distinguishes_conviction_from_acquittal_and_collects_only_personal_funds() {
    let mut fixture = fixture();
    let sponsor = sponsor(&fixture);
    // The production intake refers two strong admissible conduct exhibits plus
    // known association. The latter cannot count toward court proof.
    let opened = daily_boundary(&fixture.registry, &mut fixture.state).opened_prosecution_cases[0];
    validate_release_arrest(&fixture.state, fixture.arrest)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let mut state = fixture.state.clone();
    let mut defended = fixture;
    let defense = retain_defense(&mut defended);
    // Use the same police file in a separate branch, with active counsel the sole
    // difference relevant to the hearing. No private records are manufactured.
    let defendant = defended.defendant;
    let personal = account(
        &mut state,
        FinancialOwner::Character(defendant),
        AccountKind::StreetCash,
    );
    let hidden = account(
        &mut state,
        FinancialOwner::Character(defendant),
        AccountKind::ConcealedCash,
    );
    let employer = account(
        &mut state,
        FinancialOwner::Organization(sponsor),
        AccountKind::AccountedFunds,
    );
    let source = account(
        &mut state,
        FinancialOwner::Organization(sponsor),
        AccountKind::Settlement,
    );
    fund(&mut state, source, personal, 12000);
    fund(&mut state, source, hidden, 50000);
    fund(&mut state, source, employer, 1000000);
    let now = state.now();
    for s in [&mut state, &mut defended.state] {
        s.advance_clock(SimDuration::from_minutes(
            14 * DAY_DURATION.as_minutes() - 1,
        ));
    }
    let mut restored = restore_save(
        &defended.registry,
        build_save(&defended.registry, &state).unwrap(),
    )
    .unwrap();
    let tick = run_test_tick(&defended.registry, &mut state);
    let replay = run_test_tick(&defended.registry, &mut restored);
    assert_eq!(tick, replay);
    assert_eq!(tick.adjudicated_prosecution_cases, vec![opened]);
    assert_eq!(
        state.now().as_minutes(),
        now.as_minutes() + 14 * u64::from(DAY_DURATION.as_minutes())
    );
    let conviction = state.legal().get_prosecution_case(opened).unwrap();
    assert_eq!(conviction.status(), ProsecutionCaseStatus::Convicted);
    let judgment = conviction.judgment().unwrap();
    assert_eq!(judgment.independent_sources(), 2);
    assert_eq!(judgment.proof_score(), 8);
    assert!(judgment.defense().is_none());
    let fine = judgment.fine().cents();
    assert!(fine > 12000);
    assert_eq!(tick.court_fine_payments.len(), 1);
    assert_eq!(
        state.finance().get_account(personal).unwrap().balance(),
        Money::ZERO
    );
    assert_eq!(
        state
            .finance()
            .get_account(hidden)
            .unwrap()
            .balance()
            .cents(),
        50000
    );
    // Payroll happened first: one daily wage was debited from employer, not the fine.
    assert_eq!(
        state
            .finance()
            .get_account(employer)
            .unwrap()
            .balance()
            .cents(),
        1000000 - defended.registry.upkeep().per_member_daily().cents()
    );
    assert_eq!(
        member_judgments(&state, sponsor, defended.arrest)[0]
            .outstanding
            .cents(),
        fine - 12000 - defended.registry.upkeep().per_member_daily().cents()
    );
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    let acquitted = run_test_tick(&defended.registry, &mut defended.state);
    assert_eq!(acquitted.adjudicated_prosecution_cases, vec![opened]);
    assert!(acquitted.court_fine_payments.is_empty());
    let judgment = defended.state.legal().get_prosecution_case(opened).unwrap();
    assert_eq!(judgment.status(), ProsecutionCaseStatus::Acquitted);
    assert_eq!(judgment.judgment().unwrap().defense(), Some(defense));
    let outstanding = member_judgments(&state, sponsor, defended.arrest)[0]
        .outstanding
        .cents();
    let later = daily_boundary(&defended.registry, &mut state);
    assert_eq!(later.court_fine_payments.len(), 1);
    assert_eq!(
        member_judgments(&state, sponsor, defended.arrest)[0]
            .outstanding
            .cents(),
        outstanding - defended.registry.upkeep().per_member_daily().cents()
    );
    restore_save(
        &defended.registry,
        build_save(&defended.registry, &state).unwrap(),
    )
    .unwrap();
    restore_save(
        &defended.registry,
        build_save(&defended.registry, &defended.state).unwrap(),
    )
    .unwrap();
}
