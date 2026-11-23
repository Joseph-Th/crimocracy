//! Institutional relationship, contact disclosure and legal counsel player tests.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

fn boss_id(session: &Session<'_>) -> CharacterId {
    session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| person.supervisor().is_none())
        .expect("the authored opening has a boss")
        .id()
}

fn lawyer_id(session: &Session<'_>) -> CharacterId {
    session
        .known_character_ids()
        .into_iter()
        .find(|id| {
            session
                .state
                .world()
                .get_character(*id)
                .is_some_and(|person| person.name() == "Nora Keene")
        })
        .expect("the opening must offer an identified legal-services acquaintance")
}

#[test]
fn text_play_legal_contact_can_be_established_saved_and_terminated() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let lawyer = lawyer_id(&session);
    let boss = boss_id(&session);
    assert!(
        session
            .execute("connections")
            .unwrap()
            .0
            .contains("Cedar Street Legal")
    );
    let original = snapshot(&registry, &session);
    for command in [
        "connect char-1 char-11",  // Not an organization member
        "connect char-2 char-999", // Not known; no hidden roster access
        "connect char-2 char-2",   // Institutional contact must be external
        "contact contact-99 ask",
        "contact contact-1 end ignored",
        "retain arrest-1 contact-1 2500", // No actual known arrest
        "retain arrest-1 contact-1 -1",
        "end-counsel representation-1 withdraw",
    ] {
        assert!(session.execute(command).is_err(), "{command} must reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} changed campaign state on rejection"
        );
    }
    let created = session
        .execute(&format!(
            "connect char-{} char-{}",
            boss.raw(),
            lawyer.raw()
        ))
        .unwrap()
        .0;
    assert!(created.contains("Established Legal contact-"));
    let legal = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.contact() == lawyer)
        .unwrap()
        .id();
    assert!(
        session
            .execute("contacts")
            .unwrap()
            .0
            .contains("Cedar Street Legal")
    );
    assert!(
        session
            .execute("counsel")
            .unwrap()
            .0
            .contains("Legal channel")
    );
    assert!(
        session
            .execute(&format!("contact {} ask", legal.raw()))
            .unwrap()
            .0
            .contains("no new available information")
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("connect {} {}", boss.raw(), lawyer.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    let saved = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut resumed = Session {
        registry: &registry,
        state: saved,
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert_eq!(before, snapshot(&registry, &resumed));
    assert!(
        resumed
            .execute(&format!("contact contact-{} end", legal.raw()))
            .unwrap()
            .0
            .contains("terminated")
    );
    assert!(
        resumed
            .execute("contacts")
            .unwrap()
            .0
            .contains("Terminated")
    );
    assert!(
        resumed
            .execute(&format!("contact {} ask", legal.raw()))
            .is_err()
    );
    assert!(
        resumed
            .execute(&format!("connect {} {}", boss.raw(), lawyer.raw()))
            .unwrap()
            .0
            .contains("Established Legal")
    );
}

#[test]
fn text_play_police_disclosure_and_new_legal_channel_remain_independent() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let lawyer = lawyer_id(&session);
    let boss = boss_id(&session);
    session
        .execute(&format!("connect {} {}", boss.raw(), lawyer.raw()))
        .unwrap();
    session.execute("burgle 1 abort").unwrap();
    session.execute("advance 3h").unwrap();
    let pending = session.execute("contacts").unwrap().0;
    assert!(pending.contains("Police Active"));
    assert!(pending.contains("Legal Active"));
    assert!(pending.contains("undisclosed source(s)"));
    let police = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.kind() == ContactKind::Police)
        .unwrap()
        .id();
    let pending_count = find_pending_disclosure_sources(&session.state, police).len();
    assert!(pending_count > 0);
    let report = session
        .execute(&format!("contact {} ask-all", police.raw()))
        .unwrap()
        .0;
    assert_eq!(
        report.matches("Contact disclosed").count(),
        pending_count.min(64)
    );
    assert!(report.contains("observed minute"), "{report}");
    assert!(find_pending_disclosure_sources(&session.state, police).is_empty());
    assert!(
        session
            .execute("intelligence")
            .unwrap()
            .0
            .contains("LegalActivity")
    );
    let after = snapshot(&registry, &session);
    let lawyer_contact = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.contact() == lawyer)
        .unwrap()
        .id();
    assert!(
        session
            .execute(&format!("contact {} ask", lawyer_contact.raw()))
            .unwrap()
            .0
            .contains("no new available information")
    );
    assert_eq!(
        after,
        snapshot(&registry, &session),
        "asking a contact with no pending information must be read-only"
    );
}

#[test]
fn text_play_recent_contact_briefing_selects_latest_per_subject_without_losing_history() {
    use crimocracy::intelligence::intelligence_system::validate_record_information;
    use crimocracy::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
    };
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let contact = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.kind() == ContactKind::Police)
        .unwrap();
    let source = contact.contact();
    let contact_id = contact.id();
    let institution = contact.institution();
    let mut ids = Vec::new();
    for day in 0..3 {
        let id = validate_record_information(
            &session.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(source),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::LegalActivity,
                source_entity: None,
                subject: EntityRef::Organization(institution),
                observed_at: session.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: format!("Daily precinct summary {day}"),
            },
        )
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
        ids.push(id);
        run_tick(&registry, &mut session.state).unwrap();
    }
    let before = snapshot(&registry, &session);
    let contacts = session.execute("contacts").unwrap().0;
    assert!(
        contacts.contains("3 undisclosed source(s), 1 unseen subject/topic(s)"),
        "{contacts}"
    );
    assert_eq!(
        before,
        snapshot(&registry, &session),
        "listing contacts must not disclose anything"
    );
    let recent = session
        .execute(&format!("contact {} ask-recent", contact_id.raw()))
        .unwrap()
        .0;
    assert_eq!(recent.matches("Contact disclosed").count(), 1, "{recent}");
    assert!(recent.contains("Daily precinct summary 2"), "{recent}");
    assert!(
        recent.contains("2 undisclosed historical source(s) remain"),
        "{recent}"
    );
    assert_eq!(
        find_pending_disclosure_sources(&session.state, contact_id),
        ids[..2].iter().copied().rev().collect::<Vec<_>>()
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        find_pending_disclosure_sources(&restored, contact_id),
        ids[..2].iter().copied().rev().collect::<Vec<_>>()
    );
    let full = session
        .execute(&format!("contact {} ask-all", contact_id.raw()))
        .unwrap()
        .0;
    assert_eq!(full.matches("Contact disclosed").count(), 2, "{full}");
    assert!(find_pending_disclosure_sources(&session.state, contact_id).is_empty());
}

#[test]
fn text_play_player_can_retain_and_withdraw_counsel_in_real_custody() {
    use crimocracy::legal::LegalRepresentationStatus;
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let boss = boss_id(&session);
    let lawyer = lawyer_id(&session);
    let lieutenant = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| member.name() == "Carlo Venn")
        .unwrap()
        .id();
    assert!(
        session
            .state
            .social()
            .get_relationship(lieutenant, lawyer)
            .is_some(),
        "the player needs a lawful handler who remains available when the boss is detained"
    );
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    for _ in 0..(DAY_MINUTES * 2) {
        if !session.known_member_arrests().is_empty() {
            break;
        }
        run_tick(&registry, &mut session.state).unwrap();
    }
    let arrest = *session
        .known_member_arrests()
        .iter()
        .next()
        .expect("violent robbery must cause an actual player custody case on the fixed seed");
    let defendant = session
        .state
        .legal()
        .get_arrest(arrest)
        .unwrap()
        .character();
    assert_eq!(defendant.raw(), 2);
    assert!(
        session
            .execute("counsel")
            .unwrap()
            .0
            .contains(&format!("arrest-{}", arrest.raw()))
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("retain {} 1 5000", arrest.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));

    // The arrested boss cannot act as the institutional contact handler.
    let available = session.execute("connections").unwrap().0;
    assert!(available.contains(&format!(
        "char-{} Nora Keene | Cedar Street Legal | known to char-{} Carlo Venn",
        lawyer.raw(),
        lieutenant.raw(),
    )));
    assert!(
        !available.contains("known to char-2 Joseph Marrow"),
        "the contacts view must not offer a detained handler"
    );
    assert!(
        session
            .execute(&format!("connect 2 {}", lawyer.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    session
        .execute(&format!("connect {} {}", lieutenant.raw(), lawyer.raw()))
        .unwrap();
    let contact = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.contact() == lawyer)
        .unwrap();
    let legal_contact = contact.id();
    let provider = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(contact.institution()))
        .find(|account| account.kind() == AccountKind::LegitimateOperating)
        .unwrap()
        .id();
    let funds_before = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .filter(|account| account.kind().is_liquid())
        .map(|account| account.balance().cents() as i128)
        .sum::<i128>();
    let provider_before = session
        .state
        .finance()
        .get_account(provider)
        .unwrap()
        .balance()
        .cents();
    let saved = snapshot(&registry, &session);
    for command in [
        format!("retain arrest-{} contact-1 5000", arrest.raw()),
        format!(
            "retain arrest-{} contact-{} 0",
            arrest.raw(),
            legal_contact.raw()
        ),
        format!(
            "retain arrest-{} contact-{} 99999999",
            arrest.raw(),
            legal_contact.raw()
        ),
    ] {
        assert!(
            session.execute(&command).is_err(),
            "{command} should be rejected"
        );
        assert_eq!(saved, snapshot(&registry, &session));
    }
    let receipt = session
        .execute(&format!(
            "retain arrest-{} contact-{} 5000",
            arrest.raw(),
            legal_contact.raw()
        ))
        .unwrap()
        .0;
    assert!(receipt.contains("Retained counsel as representation-"));
    let representation = session
        .state
        .legal()
        .active_representation_for_arrest(arrest)
        .unwrap();
    assert_eq!(
        representation.origin(),
        LegalRepresentationOrigin::DirectRetention
    );
    assert_eq!(representation.status(), LegalRepresentationStatus::Active);
    assert_eq!(representation.counsel(), lawyer);
    let representation_id = representation.id();
    let funds_after = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .filter(|account| account.kind().is_liquid())
        .map(|account| account.balance().cents() as i128)
        .sum::<i128>();
    assert_eq!(funds_before - funds_after, 5_000);
    assert_eq!(
        session
            .state
            .finance()
            .get_account(provider)
            .unwrap()
            .balance()
            .cents(),
        provider_before + 5_000
    );
    let after = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!(
                "retain {} {} 5000",
                arrest.raw(),
                legal_contact.raw()
            ))
            .is_err()
    );
    assert!(
        session
            .execute(&format!(
                "end-counsel {} concluded",
                representation_id.raw()
            ))
            .is_err()
    );
    assert!(
        session
            .execute(&format!("contact {} end", legal_contact.raw()))
            .is_err(),
        "an active retainer must lock its contact"
    );
    assert_eq!(after, snapshot(&registry, &session));

    // Counsel locks termination, but not transfer to a socially connected, available handler.
    // This must preserve the representation and disclosure/contact identities.
    let replacement = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| member.id() != lieutenant && member.id() != boss)
        .expect("the fixture must have another available member")
        .id();
    let mut relationship = crimocracy::social::RelationshipDimensions::zero();
    relationship.trust = crimocracy::social::RelationshipLevel::try_new(40).unwrap();
    crimocracy::social::relationship_system::validate_set_relationship(
        &session.state,
        replacement,
        lawyer,
        relationship,
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let transfer = session
        .execute(&format!(
            "contact contact-{} reassign char-{}",
            legal_contact.raw(),
            replacement.raw(),
        ))
        .unwrap()
        .0;
    assert!(
        transfer.contains("retained counsel preserved"),
        "{transfer}"
    );
    assert_eq!(
        session
            .state
            .contacts()
            .get_contact(legal_contact)
            .unwrap()
            .handler(),
        replacement
    );
    assert_eq!(
        session
            .state
            .legal()
            .active_representation_for_arrest(arrest)
            .unwrap()
            .id(),
        representation_id
    );
    assert!(
        session
            .execute(&format!("contact {} end", legal_contact.raw()))
            .is_err()
    );
    let after_transfer = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!(
                "contact {} reassign {}",
                legal_contact.raw(),
                boss.raw()
            ))
            .is_err(),
        "a detained handler must not be selected for transfer"
    );
    assert_eq!(after_transfer, snapshot(&registry, &session));

    let restored_state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut restored = Session {
        registry: &registry,
        state: restored_state,
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert_eq!(after_transfer, snapshot(&registry, &restored));
    assert!(
        restored
            .execute("counsel")
            .unwrap()
            .0
            .contains("Nora Keene")
    );
    crimocracy::legal::arrest_system::validate_release_arrest(&restored.state, arrest)
        .unwrap()
        .commit(&mut restored.state)
        .unwrap();
    assert!(
        restored.execute("counsel").unwrap().0.contains(&format!(
            "end-counsel representation-{} concluded",
            representation_id.raw()
        )),
        "an explicitly retained lawyer needs an actionable close-out prompt after release"
    );
    restored
        .execute(&format!("end-counsel {} withdraw", representation_id.raw()))
        .unwrap();
    assert_eq!(
        restored
            .state
            .legal()
            .get_legal_representation(representation_id)
            .unwrap()
            .status(),
        LegalRepresentationStatus::Ended
    );
    restored
        .execute(&format!("contact {} end", legal_contact.raw()))
        .unwrap();
    assert!(
        restored
            .execute("contacts")
            .unwrap()
            .0
            .contains("Terminated")
    );
}

#[test]
fn counsel_defaults_to_actionable_matters_and_keeps_explicit_history_accessible() {
    use crimocracy::legal::arrest_system::validate_release_arrest;
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    session.execute("advance 1d").unwrap();
    session.execute("advance 1d").unwrap();
    let arrest = *session.known_member_arrests().iter().next().unwrap();
    let before_release = session.execute("counsel").unwrap().0;
    assert!(before_release.contains(&format!("arrest-{}", arrest.raw())));
    validate_release_arrest(&session.state, arrest)
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
    assert!(
        !crimocracy::legal::prosecution_system::member_prosecution_under_review(
            &session.state,
            session.player,
            arrest
        )
    );
    let before_views = snapshot(&registry, &session);
    let active = session.execute("counsel").unwrap().0;
    assert!(active.contains("other known arrest matter(s)"), "{active}");
    assert!(
        !active.contains(&format!("arrest-{} |", arrest.raw())),
        "{active}"
    );
    let all = session.execute("counsel all").unwrap().0;
    assert!(all.contains(&format!("arrest-{} |", arrest.raw())), "{all}");
    let selected = session
        .execute(&format!("counsel arrest-{}", arrest.raw()))
        .unwrap()
        .0;
    assert!(
        selected.contains(&format!("arrest-{} |", arrest.raw())),
        "{selected}"
    );
    assert!(session.execute("counsel arrest-999999").is_err());
    assert_eq!(before_views, snapshot(&registry, &session));
}

#[test]
fn text_play_advance_pauses_for_player_custody_before_next_decision() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    let first = session.execute("advance 1d").unwrap().0;
    assert!(
        first.contains("your operation reached a terminal outcome"),
        "{first}"
    );
    let second = session.execute("advance 1d").unwrap().0;
    assert!(second.contains("your member entered custody"), "{second}");
    assert!(second.contains("'counsel'"));
    let arrest = *session.known_member_arrests().iter().next().unwrap();
    assert_eq!(
        session.state.now(),
        session
            .state
            .legal()
            .get_arrest(arrest)
            .unwrap()
            .arrested_at(),
        "the text client must interrupt at the first custody minute"
    );
    let lawyer = lawyer_id(&session);
    session
        .execute(&format!("connect 3 {}", lawyer.raw()))
        .unwrap();
    let legal = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.contact() == lawyer)
        .unwrap()
        .id();
    let confirmation = session
        .execute(&format!("retain {} {} 5000", arrest.raw(), legal.raw()))
        .unwrap()
        .0;
    assert!(confirmation.contains("Retained counsel"));
    assert!(
        session
            .state
            .legal()
            .active_representation_for_arrest(arrest)
            .is_some()
    );

    // A custody release is another observable, time-critical personnel change.
    let release_at = session
        .state
        .legal()
        .get_arrest(arrest)
        .unwrap()
        .arrested_at()
        + registry.legal().maximum_detention();
    while session.state.now().as_minutes() + 1 < release_at.as_minutes() {
        run_tick(&registry, &mut session.state).unwrap();
    }
    // This direct-tick custody fixture can now encounter a genuine rival
    // protection demand during the long detention window. Reject such player
    // decisions before testing the independent custody-release pause contract.
    let demands: Vec<_> = session
        .state
        .decisions()
        .pending_for_recipient(session.player)
        .filter(|decision| {
            matches!(
                decision.context(),
                crimocracy::decisions::DecisionContext::ProtectionDemand(_)
            )
        })
        .map(|decision| decision.id())
        .collect();
    for demand in demands {
        session
            .execute(&format!("decide {} reject", demand.raw()))
            .unwrap();
    }
    let release = session.execute("advance 1h").unwrap().0;
    assert!(
        release.contains("your member was released from custody"),
        "{release}"
    );
    assert_eq!(session.state.now(), release_at);
}

#[test]
fn text_play_automatic_counsel_policy_acts_before_detained_member_decision() {
    use crimocracy::legal::LegalRepresentationStatus;
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let lawyer = lawyer_id(&session);
    session
        .execute(&format!("connect 3 {}", lawyer.raw()))
        .unwrap();
    session.execute("policy legal automatic").unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    let _ = session.execute("advance 1d").unwrap();
    let alert = session.execute("advance 1d").unwrap().0;
    assert!(alert.contains("entered custody"), "{alert}");
    let arrest = *session.known_member_arrests().iter().next().unwrap();
    let representation = session
        .state
        .legal()
        .active_representation_for_arrest(arrest)
        .expect("the automatic policy must actually retain an available lawyer");
    assert_eq!(representation.status(), LegalRepresentationStatus::Active);
    assert_eq!(
        representation.origin(),
        LegalRepresentationOrigin::AutomaticPolicy
    );
    assert_eq!(representation.counsel(), lawyer);
    assert_eq!(
        representation.fee(),
        registry.legal().automatic_support_retainer()
    );
    assert_eq!(
        session
            .state
            .legal()
            .get_arrest(arrest)
            .unwrap()
            .arrested_at(),
        session.state.now()
    );
    let before = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
