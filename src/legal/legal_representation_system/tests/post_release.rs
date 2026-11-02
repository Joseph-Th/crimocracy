//! Counsel retained for filed prosecution after custody has ended.

use super::*;
use crate::core::id::ProsecutionCaseId;
use crate::core::time::SimDuration;

fn open_prosecution(fixture: &mut Fixture) -> ProsecutionCaseId {
    let office = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Released-Defendant Prosecution Office".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .expect("prosecutor office should validate");
    let prosecutor = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Reviewing Attorney".to_owned(),
            organization: Some(office),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(82))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("reviewing attorney should validate");
    let evidence = fixture
        .state
        .legal()
        .get_arrest(fixture.arrest)
        .expect("arrest must persist")
        .evidence()
        .clone();
    validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: office,
            prosecutor,
            evidence,
        },
    )
    .expect("released defendant may be prosecuted")
    .commit(&mut fixture.state)
    .expect("prosecution referral should commit")
}

#[test]
fn released_defendant_without_a_filed_case_cannot_be_represented() {
    let mut fixture = fixture();
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let before = bincode::serialize(&fixture.state).expect("fixture state must serialize");
    assert_eq!(
        validate_retain_legal_representation(
            &fixture.state,
            representation_draft(&fixture, 6_000, None)
        )
        .err()
        .expect("released defendant without a case cannot retain matter-specific counsel"),
        LegalRepresentationError::NoActiveLegalMatter {
            arrest: fixture.arrest
        }
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected state must serialize"),
        before
    );
}

#[test]
fn post_release_prosecution_supports_direct_counsel_without_false_detention_knowledge() {
    let mut fixture = fixture();
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let case = open_prosecution(&mut fixture);
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let representation = retain(&mut fixture, 7_500, None);
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("direct counsel should be retained");
    assert_eq!(
        record.origin(),
        crate::legal::LegalRepresentationOrigin::DirectRetention
    );
    assert_eq!(
        fixture
            .state
            .intelligence()
            .get_information(record.information())
            .expect("retainer activity is known")
            .signal(),
        None,
        "post-release retainer must not fabricate a currently detained person"
    );
    validate_state(&fixture.state).expect("post-release retainer must be structurally valid");
    validate_invariants(&fixture.state);

    validate_decline_prosecution_case(&fixture.state, case)
        .expect("open prosecution should be declinable")
        .commit(&mut fixture.state)
        .expect("case closure should commit");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("post-release defense and closed prosecution should save"),
    )
    .expect("closed historical matter must still prove the earlier retainer was valid");
    assert_eq!(
        restored
            .legal()
            .prosecution_cases_for_arrest(fixture.arrest)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![case],
        "the restored arrest index must retain resolved prosecution episodes"
    );
    assert!(
        restored
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_some(),
        "directly retained counsel is ended explicitly rather than swept automatically"
    );
    validate_state(&restored).expect("restored defense and prosecution must validate");
    validate_invariants(&restored);
}

fn enable_automatic_defense(fixture: &mut Fixture) {
    validate_set_policy(
        &fixture.registry,
        &fixture.state,
        fixture.sponsor,
        PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
    )
    .expect("automatic support policy should validate")
    .commit(&fixture.registry, &mut fixture.state)
    .expect("automatic support policy should commit");
}

#[test]
fn completed_automatic_defense_concludes_when_new_retainers_cannot_allocate() {
    let mut fixture = fixture();
    enable_automatic_defense(&mut fixture);
    let retained = apply_automatic_legal_support(&fixture.registry, &mut fixture.state)
        .expect("initial legal support should validate")
        .retained;
    assert_eq!(retained.len(), 1);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("first matter should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");

    let second_defendant = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "New Defendant Awaiting Support".to_owned(),
            organization: Some(fixture.sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second organization member should validate");
    let draft = arrest_draft_for_character(&mut fixture, second_defendant, "New counsel matter");
    let second_arrest = validate_arrest(&fixture.registry, &fixture.state, draft)
        .expect("second arrest should validate")
        .commit(&mut fixture.state)
        .expect("second arrest should commit");
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::LegalRepresentation, u32::MAX);

    let outcome = apply_automatic_legal_support(&fixture.registry, &mut fixture.state)
        .expect("an exhausted new retainer cannot prevent a completed matter's closure");
    assert_eq!(outcome.concluded, 1);
    assert!(outcome.retained.is_empty());
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none()
    );
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(second_arrest)
            .is_none()
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_legal_representation(retained[0])
            .expect("concluded defense should remain historical")
            .end_reason(),
        Some(LegalRepresentationEndReason::MatterConcluded)
    );
    validate_state(&fixture.state).expect("isolated legal closure must preserve state");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("closure should save"),
    )
    .expect("closure must survive restore");
    assert!(
        restored
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none()
    );
    validate_invariants(&restored);
}

#[test]
fn automatic_support_does_not_choose_a_prefix_when_retainer_ids_are_shared() {
    let mut fixture = fixture();
    enable_automatic_defense(&mut fixture);
    let second_defendant = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second Defendant Awaiting Support".to_owned(),
            organization: Some(fixture.sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second organization member should validate");
    let draft = arrest_draft_for_character(&mut fixture, second_defendant, "Second counsel matter");
    let second_arrest = validate_arrest(&fixture.registry, &fixture.state, draft)
        .expect("second custody should validate")
        .commit(&mut fixture.state)
        .expect("second custody should commit");
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::LegalRepresentation, u32::MAX - 1);
    let before = bincode::serialize(&fixture.state).expect("two-matter fixture should serialize");

    let outcome = apply_automatic_legal_support(&fixture.registry, &mut fixture.state)
        .expect("shared retainer-ID shortfall must be a no-op");
    assert!(outcome.retained.is_empty());
    assert_eq!(outcome.concluded, 0);
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected legal cohort should serialize"),
        before,
        "scarce shared IDs cannot assign counsel only to the first arrested member"
    );
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none()
    );
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(second_arrest)
            .is_none()
    );
    validate_state(&fixture.state).expect("rejected cohort must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn automatic_policy_retains_counsel_for_released_defendant_with_prosecution_only() {
    let mut fixture = fixture();
    enable_automatic_defense(&mut fixture);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("custody should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let first_case = open_prosecution(&mut fixture);
    let second_case = open_prosecution(&mut fixture);
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_with_open_prosecutions()
            .collect::<Vec<_>>(),
        vec![fixture.arrest],
        "two prosecutor offices must not produce two retention candidates"
    );
    assert!(!fixture.state.legal().has_detained_arrests());

    let payer_before = fixture
        .state
        .finance()
        .get_account(fixture.payer)
        .expect("sponsor reserves must persist")
        .balance();
    let outcome = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(outcome.automatic_legal_support.len(), 1);
    let representation = outcome.automatic_legal_support[0];
    let retained = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("automatic defense must persist");
    assert_eq!(retained.arrest(), fixture.arrest);
    assert_eq!(
        retained.origin(),
        crate::legal::LegalRepresentationOrigin::AutomaticPolicy
    );
    assert_eq!(
        fixture
            .state
            .intelligence()
            .get_information(retained.information())
            .expect("retention knowledge must persist")
            .signal(),
        None,
        "representation begun after release is not evidence of current detention"
    );
    let fee = fixture.registry.legal().automatic_support_retainer();
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.payer)
            .expect("sponsor reserves must persist")
            .balance(),
        payer_before
            .checked_sub(fee)
            .expect("fixture reserves exceed retainer")
    );
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("post-release automatic representation should save"),
    )
    .expect("post-release automatic representation should restore");
    fixture.state = restored;

    validate_decline_prosecution_case(&fixture.state, first_case)
        .expect("first reviewing case should be declinable")
        .commit(&mut fixture.state)
        .expect("first case should close");
    let reviewing = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(reviewing.concluded_automatic_legal_support, 0);
    assert!(reviewing.automatic_legal_support.is_empty());
    assert_eq!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .map(|record| record.id()),
        Some(representation)
    );

    validate_decline_prosecution_case(&fixture.state, second_case)
        .expect("remaining reviewing case should be declinable")
        .commit(&mut fixture.state)
        .expect("final case should close");
    let finished = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(finished.concluded_automatic_legal_support, 1);
    assert!(finished.automatic_legal_support.is_empty());
    validate_terminate_contact(&fixture.state, fixture.contact)
        .expect("finished automatic defense should release its legal contact");
    validate_state(&fixture.state).expect("completed automatic defense must validate");
    validate_invariants(&fixture.state);
}

#[test]
fn automatic_policy_does_not_spend_on_released_defendant_without_open_prosecution() {
    let mut fixture = fixture();
    enable_automatic_defense(&mut fixture);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("custody should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let before = bincode::serialize(&fixture.state).expect("stable legal state must serialize");
    let outcome = apply_automatic_legal_support(&fixture.registry, &mut fixture.state)
        .expect("quiet automatic policy should be an ordinary no-op");
    assert!(outcome.retained.is_empty());
    assert_eq!(outcome.concluded, 0);
    assert_eq!(
        bincode::serialize(&fixture.state).expect("quiet legal state must serialize"),
        before
    );
}

#[test]
fn automatic_policy_prioritizes_pending_detainee_over_older_released_case() {
    let mut fixture = fixture();
    enable_automatic_defense(&mut fixture);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("first custody should be releasable")
        .commit(&mut fixture.state)
        .expect("first release should commit");
    open_prosecution(&mut fixture);
    let second_defendant = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Newly Detained Associate".to_owned(),
            organization: Some(fixture.sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second member should validate");
    let second_arrest_draft =
        arrest_draft_for_character(&mut fixture, second_defendant, "Newer inquiry");
    let second_arrest = validate_arrest(&fixture.registry, &fixture.state, second_arrest_draft)
        .expect("second arrest should validate")
        .commit(&mut fixture.state)
        .expect("second arrest should commit");

    // Leave exactly one flat retainer in sponsor liquidity. Observing selection order is not
    // enough: a released case must not consume the money needed by a pending custody decision.
    let settlement = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(fixture.sponsor))
        .find(|account| account.kind() == AccountKind::Settlement)
        .expect("fixture includes an organization settlement counterparty")
        .id();
    let fee = fixture.registry.legal().automatic_support_retainer();
    let balance = fixture
        .state
        .finance()
        .get_account(fixture.payer)
        .expect("payer account must persist")
        .balance();
    let excess = balance
        .checked_sub(fee)
        .expect("fixture initially has more than one retainer");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Reserve only one automatic legal retainer".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.payer,
                    amount: excess.checked_neg().expect("positive reserve must negate"),
                },
                LedgerPosting {
                    account: settlement,
                    amount: excess,
                },
            ],
            authorization: None,
        },
    )
    .expect("reserve reduction should be valid")
    .commit(&mut fixture.state)
    .expect("reserve reduction should commit");

    let outcome = apply_automatic_legal_support(&fixture.registry, &mut fixture.state)
        .expect("available funding must go to the higher-priority matter");
    assert_eq!(outcome.retained.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_legal_representation(outcome.retained[0])
            .expect("first candidate should persist")
            .arrest(),
        second_arrest,
        "the detainee with an upcoming informant decision must take funding priority"
    );
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none(),
        "the released case must remain unrepresented until a later funding opportunity"
    );
    validate_state(&fixture.state).expect("concurrent legal matters must validate");
    validate_invariants(&fixture.state);
}

#[test]
fn prosecution_closure_stales_uncommitted_post_release_retainer_atomically() {
    let mut fixture = fixture();
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let case = open_prosecution(&mut fixture);
    let held = validate_retain_legal_representation(
        &fixture.state,
        representation_draft(&fixture, 7_500, None),
    )
    .expect("open prosecution should permit direct retainer validation");
    validate_decline_prosecution_case(&fixture.state, case)
        .expect("case disposition should validate")
        .commit(&mut fixture.state)
        .expect("case disposition should commit");
    let before = bincode::serialize(&fixture.state).expect("closed case must serialize");
    assert_eq!(
        held.commit(&mut fixture.state)
            .expect_err("outdated retainer cannot charge a lawyer after matter closure"),
        LegalRepresentationError::NoActiveLegalMatter {
            arrest: fixture.arrest
        }
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("failed retainer must not mutate state"),
        before
    );
    validate_state(&fixture.state).expect("rejected retainer state must validate");
    validate_invariants(&fixture.state);
}

#[test]
fn matter_concluded_cannot_be_claimed_while_custody_or_prosecution_remains_active() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    let still_detained = validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::MatterConcluded,
    )
    .err()
    .expect("a retained defendant still in custody has an active legal matter");
    let expected = LegalRepresentationError::MatterStillActive {
        representation,
        arrest: fixture.arrest,
    };
    assert_eq!(still_detained, expected);

    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("custody should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let case = open_prosecution(&mut fixture);
    let before = bincode::serialize(&fixture.state).expect("open matter must serialize");
    assert_eq!(
        validate_end_legal_representation(
            &fixture.state,
            representation,
            LegalRepresentationEndReason::MatterConcluded,
        )
        .err()
        .expect("an open prosecution still needs defense"),
        expected
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected end must serialize"),
        before
    );
    // Withdrawal is a discretionary decision, not a claim that the legal matter is closed.
    validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::SponsorWithdrawn,
    )
    .expect("sponsor can withdraw defense before prosecution ends")
    .commit(&mut fixture.state)
    .expect("sponsor withdrawal should commit");
    assert!(
        fixture
            .state
            .legal()
            .has_open_prosecution_case_for_arrest(fixture.arrest)
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("open prosecution remains separate")
            .status(),
        crate::legal::ProsecutionCaseStatus::Reviewing
    );
    validate_state(&fixture.state).expect("withdrawal must keep legal history valid");
    validate_invariants(&fixture.state);
}

#[test]
fn new_prosecution_stales_prevalidated_matter_conclusion_without_partial_end() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("custody should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let held = validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::MatterConcluded,
    )
    .expect("released custody without a filed prosecution is a concluded matter");
    open_prosecution(&mut fixture);
    let before = bincode::serialize(&fixture.state).expect("newly filed case must serialize");
    assert_eq!(
        held.commit(&mut fixture.state)
            .expect_err("filing a prosecution must stale an earlier conclusion token"),
        LegalRepresentationError::MatterStillActive {
            representation,
            arrest: fixture.arrest,
        }
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected conclusion must not mutate"),
        before
    );
    validate_state(&fixture.state).expect("stale conclusion must preserve validity");
    validate_invariants(&fixture.state);
}

#[test]
fn matter_conclusion_after_case_disposition_survives_restore() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("custody should be releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let case = open_prosecution(&mut fixture);
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    validate_decline_prosecution_case(&fixture.state, case)
        .expect("open prosecution should be declinable")
        .commit(&mut fixture.state)
        .expect("case closure should commit");
    validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::MatterConcluded,
    )
    .expect("completed detention and prosecution justify final defense closure")
    .commit(&mut fixture.state)
    .expect("final defense closure should commit");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("completed legal matter should save"),
    )
    .expect("completed legal matter should restore");
    assert_eq!(
        restored
            .legal()
            .get_legal_representation(representation)
            .expect("historical representation must persist")
            .end_reason(),
        Some(LegalRepresentationEndReason::MatterConcluded)
    );
    validate_state(&restored).expect("restored concluded matter must remain valid");
    validate_invariants(&restored);
}
