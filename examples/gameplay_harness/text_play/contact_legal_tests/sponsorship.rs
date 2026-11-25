//! Sponsor ownership and paid counsel across actual personnel transfers.

use super::*;

#[test]
fn sponsored_retainer_remains_manageable_after_member_defection() {
    use crimocracy::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crimocracy::legal::investigation_system::{
        validate_add_evidence, validate_open_investigation,
    };
    use crimocracy::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft, LegalRepresentationStatus,
    };
    use crimocracy::social::relationship_system::validate_set_relationship;
    use crimocracy::social::{RelationshipDimensions, RelationshipLevel};
    use crimocracy::world::world_system::{insert_character, insert_organization};
    use crimocracy::world::{
        AutonomyLevel, CapabilityKind, CharacterDraft, DriveKind, OrganizationDraft,
        OrganizationKind, Rating, TraitKind,
    };

    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let boss = boss_id(&session);
    let lawyer = lawyer_id(&session);
    session
        .execute(&format!("connect {} {}", boss.raw(), lawyer.raw()))
        .unwrap();
    let contact = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.contact() == lawyer)
        .unwrap()
        .id();
    let rival = insert_organization(
        &registry,
        &mut session.state,
        OrganizationDraft {
            name: "Defection fixture crew".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let rating = |n| Rating::try_new(n).unwrap();
    let recruiter = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Rival recruiter".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(90))]),
            traits: BTreeSet::from([TraitKind::Charismatic]),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let defendant = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Departing client".into(),
            organization: Some(session.player),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::from([TraitKind::EasilyFrightened]),
            drives: BTreeMap::from([(DriveKind::Safety, rating(90))]),
        },
    )
    .unwrap();
    let level = |n| RelationshipLevel::try_new(n).unwrap();
    validate_set_relationship(
        &session.state,
        defendant,
        recruiter,
        RelationshipDimensions {
            trust: level(100),
            respect: level(100),
            fear: level(0),
            affection: level(100),
            dependence: level(100),
            resentment: level(0),
            debt: level(100),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let police = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.kind() == ContactKind::Police)
        .unwrap()
        .institution();
    let detain = |state: &mut AppState, title: &str| {
        let investigation = validate_open_investigation(
            state,
            InvestigationDraft {
                owner: police,
                title: title.into(),
                subjects: BTreeSet::from([EntityRef::Character(defendant)]),
            },
        )
        .unwrap()
        .commit(state)
        .unwrap();
        let mut evidence = BTreeSet::new();
        for kind in [EvidenceKind::Document, EvidenceKind::FinancialRecord] {
            evidence.insert(
                validate_add_evidence(
                    state,
                    EvidenceDraft {
                        investigation,
                        custodian: police,
                        subject: EntityRef::Character(defendant),
                        origin: None,
                        kind,
                        strength: EvidenceStrength::Strong,
                        reliability: EvidenceReliability::HighlyReliable,
                        admissibility: Admissibility::Admissible,
                        discovered_at: state.now(),
                    },
                )
                .unwrap()
                .commit(state)
                .unwrap(),
            );
        }
        validate_arrest(
            &registry,
            state,
            ArrestDraft {
                character: defendant,
                investigation,
                evidence,
            },
        )
        .unwrap()
        .commit(state)
        .unwrap()
    };
    let arrest = detain(&mut session.state, "Departure fixture custody");
    session
        .execute(&format!("retain {} {} 5000", arrest.raw(), contact.raw()))
        .unwrap();
    let representation = session
        .state
        .legal()
        .active_representation_for_arrest(arrest)
        .unwrap()
        .id();
    validate_release_arrest(&session.state, arrest)
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
    let attempt = validate_recruitment_attempt(
        &registry,
        &session.state,
        RecruitmentDraft {
            target_organization: rival,
            recruiter,
            candidate: defendant,
            approach: RecruitmentApproach::Protection,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    assert_eq!(
        session
            .state
            .recruitment()
            .get_attempt(attempt)
            .unwrap()
            .outcome(),
        crimocracy::recruitment::RecruitmentOutcome::Accepted
    );
    assert!(!session.known_member_arrests().contains(&arrest));
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let before = snapshot(&registry, &session);
    let summary = session.execute("counsel").unwrap().0;
    assert!(summary.contains("Departing client"), "{summary}");
    assert!(summary.contains("former member"), "{summary}");
    assert!(
        summary.contains(&format!("representation-{}", representation.raw())),
        "{summary}"
    );
    assert!(
        session
            .execute(&format!("counsel arrest-{}", arrest.raw()))
            .is_ok()
    );
    assert!(
        session
            .execute(&format!("retain {} {} 5000", arrest.raw(), contact.raw()))
            .is_err()
    );
    assert!(
        session
            .execute(&format!("contact {} end", contact.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    // An actual open prosecution of the former member remains relevant to
    // the paid engagement. The sponsor must not be told it is concluded.
    let office = insert_organization(
        &registry,
        &mut session.state,
        OrganizationDraft {
            name: "Departure fixture prosecutor".into(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .unwrap();
    let prosecutor = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Reviewing prosecutor".into(),
            organization: Some(office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    use crimocracy::legal::prosecution_system::{
        validate_decline_prosecution_case, validate_open_prosecution_case,
    };
    let case = validate_open_prosecution_case(
        &session.state,
        crimocracy::legal::ProsecutionCaseDraft {
            arrest,
            prosecutor_office: office,
            prosecutor,
            evidence: session
                .state
                .legal()
                .get_arrest(arrest)
                .unwrap()
                .evidence()
                .clone(),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let before = snapshot(&registry, &session);
    let view = session.execute("counsel").unwrap().0;
    assert!(view.contains("Prosecutorial review remains open"), "{view}");
    assert!(!view.contains("no longer needs active defense"), "{view}");
    assert!(
        session
            .execute(&format!("end-counsel {} concluded", representation.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    validate_decline_prosecution_case(&session.state, case)
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
    session
        .execute(&format!("end-counsel {} concluded", representation.raw()))
        .unwrap();
    assert_eq!(
        session
            .state
            .legal()
            .get_legal_representation(representation)
            .unwrap()
            .status(),
        LegalRepresentationStatus::Ended
    );
    session
        .execute(&format!("contact {} end", contact.raw()))
        .unwrap();

    // A subsequent rival-funded engagement stays with the rival if the
    // defendant rejoins us. Current membership reveals the person's legal
    // matter, not the previous sponsor's private bill or management ID.
    let mut trust = RelationshipDimensions::zero();
    trust.trust = level(100);
    trust.respect = level(100);
    trust.affection = level(100);
    trust.dependence = level(100);
    trust.debt = level(100);
    for (from, to) in [(recruiter, lawyer), (defendant, boss)] {
        validate_set_relationship(&session.state, from, to, trust)
            .unwrap()
            .commit(&mut session.state)
            .unwrap();
    }
    let foreign_contact = validate_establish_contact(
        &session.state,
        InstitutionalContactDraft {
            sponsor: rival,
            handler: recruiter,
            contact: lawyer,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let provider = session
        .state
        .contacts()
        .get_contact(foreign_contact)
        .unwrap()
        .institution();
    let provider_account = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(provider))
        .find(|account| account.kind() == AccountKind::LegitimateOperating)
        .unwrap()
        .id();
    let payer = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let fee = Money::from_cents(7_600);
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Rival retainer funding".into(),
            authorization: None,
            postings: vec![
                LedgerPosting {
                    account: settlement,
                    amount: fee.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: payer,
                    amount: fee,
                },
            ],
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    run_tick(&registry, &mut session.state).unwrap();
    let foreign_arrest = detain(&mut session.state, "Separate rival custody");
    let foreign_representation = validate_retain_legal_representation(
        &session.state,
        LegalRepresentationDraft {
            arrest: foreign_arrest,
            sponsor: rival,
            contact: foreign_contact,
            fee,
            payer_accounts: BTreeSet::from([payer]),
            provider_account,
            authorization: None,
            origin: LegalRepresentationOrigin::DirectRetention,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    validate_release_arrest(&session.state, foreign_arrest)
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
    let return_attempt = validate_recruitment_attempt(
        &registry,
        &session.state,
        RecruitmentDraft {
            target_organization: session.player,
            recruiter: boss,
            candidate: defendant,
            approach: RecruitmentApproach::Protection,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    assert_eq!(
        session
            .state
            .recruitment()
            .get_attempt(return_attempt)
            .unwrap()
            .outcome(),
        crimocracy::recruitment::RecruitmentOutcome::Accepted
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let before = snapshot(&registry, &session);
    let view = session
        .execute(&format!("counsel arrest-{}", foreign_arrest.raw()))
        .unwrap()
        .0;
    assert!(
        view.contains("Existing counsel retained outside your organization"),
        "{view}"
    );
    assert!(
        !view.contains(&format!("representation-{}", foreign_representation.raw())),
        "{view}"
    );
    assert!(!view.contains("7600 cents"), "{view}");
    assert!(!view.contains("end-counsel"), "{view}");
    assert!(
        session
            .execute(&format!(
                "end-counsel {} withdraw",
                foreign_representation.raw()
            ))
            .is_err()
    );
    assert!(
        session
            .execute(&format!(
                "retain {} {} 5000",
                foreign_arrest.raw(),
                contact.raw()
            ))
            .unwrap_err()
            .contains("outside your organization")
    );
    assert_eq!(before, snapshot(&registry, &session));
    restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
}
