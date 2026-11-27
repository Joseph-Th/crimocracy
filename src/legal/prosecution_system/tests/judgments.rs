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
fn due_hearing_refers_decisive_evidence_before_rendering_verdict() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    fixture.state.advance_clock(SimDuration::from_minutes(
        14 * DAY_DURATION.as_minutes() - 1,
    ));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();

    // The filed document and association screen successfully, but cannot
    // convict. The police already hold an independent strong financial exhibit.
    let tick = run_test_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(tick, run_test_tick(&fixture.registry, &mut restored));
    assert!(tick.adjudicated_prosecution_cases.is_empty());
    assert_eq!(tick.prosecution_supplements.len(), 1);
    let reviewing = fixture.state.legal().get_prosecution_case(case).unwrap();
    assert_eq!(reviewing.status(), ProsecutionCaseStatus::Reviewing);
    assert!(
        reviewing
            .evidence()
            .contains(&fixture.supplemental_evidence)
    );

    // Once the file can convict, further exhibits must not indefinitely delay
    // the scheduled hearing. A prosecutor still takes only one action per day.
    for state in [&mut fixture.state, &mut restored] {
        add_evidence(
            state,
            fixture.police,
            fixture.investigation,
            fixture.defendant,
            EvidenceKind::Surveillance,
        );
    }
    let tick = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(tick, daily_boundary(&fixture.registry, &mut restored));
    assert_eq!(tick.adjudicated_prosecution_cases, vec![case]);
    assert!(tick.prosecution_supplements.is_empty());
    let verdict = fixture.state.legal().get_prosecution_case(case).unwrap();
    assert_eq!(verdict.status(), ProsecutionCaseStatus::Convicted);
    assert_eq!(verdict.judgment().unwrap().proof_score(), 8);
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn due_hearing_takes_priority_over_supplementing_a_newer_review() {
    let mut fixture = fixture();
    // The first arrest is older, but its review opens later. A prosecutor with
    // both matters must not postpone an already prepared hearing to file an
    // ordinary exhibit for the newer review.
    let other = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second defendant awaiting hearing".into(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let other_arrest = detain_character(&mut fixture, other, "Second defendant");
    let other_record = fixture.state.legal().get_arrest(other_arrest).unwrap();
    let investigation = other_record.investigation();
    let mut evidence = other_record.evidence().clone();
    evidence.insert(add_evidence(
        &mut fixture.state,
        fixture.police,
        investigation,
        other,
        EvidenceKind::FinancialRecord,
    ));
    let hearing = validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: other_arrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    for arrest in [fixture.arrest, other_arrest] {
        validate_release_arrest(&fixture.state, arrest)
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap();
    }
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(13 * DAY_DURATION.as_minutes()));
    let newer = open_case(&mut fixture);
    let before = fixture
        .state
        .legal()
        .get_prosecution_case(newer)
        .unwrap()
        .evidence()
        .clone();
    let tick = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.adjudicated_prosecution_cases, vec![hearing]);
    assert!(tick.prosecution_supplements.is_empty());
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(hearing)
            .unwrap()
            .status(),
        ProsecutionCaseStatus::Convicted
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(newer)
            .unwrap()
            .evidence(),
        &before
    );
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    let next = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(next, daily_boundary(&fixture.registry, &mut restored));
    assert_eq!(next.prosecution_supplements.len(), 1);
    assert!(
        fixture
            .state
            .legal()
            .get_prosecution_case(newer)
            .unwrap()
            .evidence()
            .contains(&fixture.supplemental_evidence)
    );
}

#[test]
fn prosecution_supplements_rotate_to_the_longest_waiting_review() {
    let mut fixture = fixture();
    let first = open_case(&mut fixture);
    let other = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second defendant with pending evidence".into(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let arrest = detain_character(&mut fixture, other, "Second defendant");
    let record = fixture.state.legal().get_arrest(arrest).unwrap();
    let investigation = record.investigation();
    let evidence = record.evidence().clone();
    let second = validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let waiting = add_evidence(
        &mut fixture.state,
        fixture.police,
        investigation,
        other,
        EvidenceKind::FinancialRecord,
    );
    let tick = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.prosecution_supplements.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_referral(tick.prosecution_supplements[0])
            .unwrap()
            .prosecution_case(),
        first
    );
    // The first matter has another real exhibit on the next day. It must not
    // consume the second day's sole prosecutor slot as well.
    let additional = add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.defendant,
        EvidenceKind::Document,
    );
    let tick = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.prosecution_supplements.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_referral(tick.prosecution_supplements[0])
            .unwrap()
            .prosecution_case(),
        second
    );
    assert!(
        fixture
            .state
            .legal()
            .get_prosecution_case(second)
            .unwrap()
            .evidence()
            .contains(&waiting)
    );
    assert!(
        !fixture
            .state
            .legal()
            .get_prosecution_case(first)
            .unwrap()
            .evidence()
            .contains(&additional)
    );
    let tick = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.prosecution_supplements.len(), 1);
    assert!(
        fixture
            .state
            .legal()
            .get_prosecution_case(first)
            .unwrap()
            .evidence()
            .contains(&additional)
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn waiting_initial_intake_rotates_with_ordinary_supplements() {
    let mut fixture = fixture();
    let first = open_case(&mut fixture);
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let other = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Defendant waiting for first review".into(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let waiting = detain_character(&mut fixture, other, "Waiting defendant");
    let first_day = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(first_day.prosecution_supplements.len(), 1);
    assert!(first_day.opened_prosecution_cases.is_empty());
    let additional = add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.defendant,
        EvidenceKind::Surveillance,
    );
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    let second_day = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(second_day, daily_boundary(&fixture.registry, &mut restored));
    assert_eq!(
        second_day.opened_prosecution_cases.len(),
        1,
        "a new exhibit must not jump ahead of an older unserved arrest again"
    );
    assert!(second_day.prosecution_supplements.is_empty());
    let opened = fixture
        .state
        .legal()
        .get_prosecution_case(second_day.opened_prosecution_cases[0])
        .unwrap();
    assert_eq!(opened.arrest(), waiting);
    let third_day = daily_boundary(&fixture.registry, &mut fixture.state);
    assert_eq!(third_day, daily_boundary(&fixture.registry, &mut restored));
    assert_eq!(third_day.prosecution_supplements.len(), 1);
    assert!(
        fixture
            .state
            .legal()
            .get_prosecution_case(first)
            .unwrap()
            .evidence()
            .contains(&additional)
    );
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

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
    // A new custody episode can use one genuinely new source alongside the
    // existing corroboration, but the court cannot reuse adjudicated proof.
    let mut rearrested = fixture.state.clone();
    rearrested.advance_clock(SimDuration::ONE_MINUTE);
    let mut combined_evidence = evidence.clone();
    combined_evidence.insert(add_evidence(
        &mut rearrested,
        fixture.police,
        fixture.investigation,
        fixture.defendant,
        EvidenceKind::Surveillance,
    ));
    let rearrest = validate_arrest(
        &fixture.registry,
        &rearrested,
        ArrestDraft {
            character: fixture.defendant,
            investigation: fixture.investigation,
            evidence: combined_evidence.clone(),
        },
    )
    .unwrap()
    .commit(&mut rearrested)
    .unwrap();
    assert_ne!(rearrest, fixture.arrest);
    validate_release_arrest(&rearrested, rearrest)
        .unwrap()
        .commit(&mut rearrested)
        .unwrap();
    let review = validate_open_prosecution_case(
        &rearrested,
        ProsecutionCaseDraft {
            arrest: rearrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence: combined_evidence,
        },
    )
    .unwrap()
    .commit(&mut rearrested)
    .unwrap();
    validate_transition_investigation(
        &rearrested,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .unwrap()
    .commit(&mut rearrested)
    .unwrap();
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &rearrested).unwrap(),
    )
    .unwrap();
    for state in [&mut rearrested, &mut restored] {
        // The indexed prior judgment must be found across arrest IDs.
        let tick = daily_boundary(&fixture.registry, state);
        assert!(tick.adjudicated_prosecution_cases.is_empty());
        assert_eq!(tick.declined_prosecution_cases, vec![review]);
        assert_eq!(
            state.legal().get_prosecution_case(review).unwrap().status(),
            ProsecutionCaseStatus::Declined
        );
        assert_eq!(
            state
                .legal()
                .get_prosecution_case(first)
                .unwrap()
                .judgment()
                .unwrap()
                .fine(),
            old_fine
        );
    }
    assert_eq!(
        bincode::serialize(&rearrested).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &rearrested).unwrap(),
    )
    .unwrap();

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
        &fixture.registry,
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
fn court_fine_uses_canonical_personal_payment_order() {
    let mut fixture = fixture();
    let sponsor = sponsor(&fixture);
    let case = daily_boundary(&fixture.registry, &mut fixture.state).opened_prosecution_cases[0];
    validate_release_arrest(&fixture.state, fixture.arrest)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let owner = FinancialOwner::Character(fixture.defendant);
    let accounted = account(&mut fixture.state, owner, AccountKind::AccountedFunds);
    let small = account(&mut fixture.state, owner, AccountKind::StreetCash);
    let large = account(&mut fixture.state, owner, AccountKind::StreetCash);
    let source = account(
        &mut fixture.state,
        FinancialOwner::Organization(sponsor),
        AccountKind::Settlement,
    );
    fund(&mut fixture.state, source, accounted, 1_000_000);
    fund(&mut fixture.state, source, small, 500);
    fund(&mut fixture.state, source, large, 1_000_000);
    fixture.state.advance_clock(SimDuration::from_minutes(
        14 * DAY_DURATION.as_minutes() - 1,
    ));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    let tick = run_test_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(tick, run_test_tick(&fixture.registry, &mut restored));
    assert_eq!(tick.adjudicated_prosecution_cases, vec![case]);
    assert_eq!(tick.court_fine_payments.len(), 1);
    let state = &fixture.state;
    let fine = state
        .legal()
        .get_prosecution_case(case)
        .unwrap()
        .judgment()
        .unwrap()
        .fine();
    assert!(fine > Money::ZERO && fine < Money::from_cents(1_000_000));
    let payment = state
        .finance()
        .get_transaction(tick.court_fine_payments[0])
        .unwrap();
    assert_eq!(payment.postings().len(), 2);
    assert_eq!(
        payment.postings()[0],
        LedgerPosting {
            account: large,
            amount: fine.checked_neg().unwrap(),
        }
    );
    assert_eq!(
        state
            .finance()
            .get_account(accounted)
            .unwrap()
            .balance()
            .cents(),
        1_000_000
    );
    assert_eq!(
        state
            .finance()
            .get_account(small)
            .unwrap()
            .balance()
            .cents(),
        500
    );
    assert_eq!(
        state
            .finance()
            .get_account(large)
            .unwrap()
            .balance()
            .cents(),
        1_000_000 - fine.cents()
    );
    assert_eq!(state.legal().outstanding_fine(case), Money::ZERO);
    assert_eq!(
        bincode::serialize(state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn court_hearing_distinguishes_conviction_from_acquittal_and_collects_only_personal_funds() {
    let mut fixture = fixture();
    let sponsor = sponsor(&fixture);
    // The production intake refers two strong admissible conduct exhibits plus
    // known association. The latter cannot count toward court proof.
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_awaiting_initial_review()
            .map(|a| a.id())
            .collect::<Vec<_>>(),
        vec![fixture.arrest]
    );
    let pending_restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        pending_restored
            .legal()
            .arrests_awaiting_initial_review()
            .map(|a| a.id())
            .collect::<Vec<_>>(),
        vec![fixture.arrest]
    );
    let opened = daily_boundary(&fixture.registry, &mut fixture.state).opened_prosecution_cases[0];
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_awaiting_initial_review()
            .count(),
        0
    );
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
    let notice = state
        .reports()
        .reports_for(sponsor)
        .find(|report| report.title() == "Court conviction")
        .expect("the member's criminal organization should receive the actual verdict");
    let summary = &notice.entries()[0].summary;
    assert!(
        summary.contains(&format!(
            "personal fine of {}",
            crate::finance::helpers::format_money_cents(fine)
        )),
        "a court-ordered fine must be rendered as money, not diagnostic cents: {summary}"
    );
    assert!(!summary.contains(" cents"));
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
    assert_eq!(defended.state.legal().unpaid_fines().count(), 0);
    assert_eq!(
        defended
            .state
            .legal()
            .arrests_awaiting_initial_review()
            .count(),
        0
    );
    let outstanding = member_judgments(&state, sponsor, defended.arrest)[0]
        .outstanding
        .cents();
    assert_eq!(
        state.legal().unpaid_fines().collect::<Vec<_>>(),
        vec![(opened, Money::from_cents(outstanding))]
    );
    let later = daily_boundary(&defended.registry, &mut state);
    assert_eq!(later.court_fine_payments.len(), 1);
    assert_eq!(
        member_judgments(&state, sponsor, defended.arrest)[0]
            .outstanding
            .cents(),
        outstanding - defended.registry.upkeep().per_member_daily().cents()
    );
    let mut restored = restore_save(
        &defended.registry,
        build_save(&defended.registry, &state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state.legal().unpaid_fines().collect::<Vec<_>>(),
        restored.legal().unpaid_fines().collect::<Vec<_>>()
    );
    let remaining = state.legal().outstanding_fine(opened).cents();
    assert!(remaining > 0);
    for branch in [&mut state, &mut restored] {
        fund(branch, source, personal, remaining);
        let paid = daily_boundary(&defended.registry, branch);
        assert_eq!(paid.court_fine_payments.len(), 1);
        assert_eq!(branch.legal().unpaid_fines().count(), 0);
        assert_eq!(
            member_judgments(branch, sponsor, defended.arrest)[0].outstanding,
            Money::ZERO
        );
        let history = branch
            .legal()
            .get_prosecution_case(opened)
            .unwrap()
            .judgment()
            .unwrap()
            .payments()
            .clone();
        assert!(history.len() >= 3);
        let next = daily_boundary(&defended.registry, branch);
        assert!(next.court_fine_payments.is_empty());
        assert_eq!(
            branch
                .legal()
                .get_prosecution_case(opened)
                .unwrap()
                .judgment()
                .unwrap()
                .payments(),
            &history
        );
        assert_eq!(branch.legal().arrests_awaiting_initial_review().count(), 0);
        *branch = restore_save(
            &defended.registry,
            build_save(&defended.registry, branch).unwrap(),
        )
        .unwrap();
        assert_eq!(branch.legal().unpaid_fines().count(), 0);
    }
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    let acquitted_restored = restore_save(
        &defended.registry,
        build_save(&defended.registry, &defended.state).unwrap(),
    )
    .unwrap();
    assert_eq!(acquitted_restored.legal().unpaid_fines().count(), 0);
    assert_eq!(
        acquitted_restored
            .legal()
            .arrests_awaiting_initial_review()
            .count(),
        0
    );
}
