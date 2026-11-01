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
