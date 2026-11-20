//! A real police investigator's case-witness knowledge must traverse its
//! institutional contact before a player may authorize witness pressure.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn police_contact_disclosure_provides_typed_witness_basis_for_player_operation() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let police_contact = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|contact| contact.kind() == ContactKind::Police)
        .unwrap();
    let detective = police_contact.contact();
    let contact_id = police_contact.id();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    session.execute("advance 1d").unwrap();
    let private = session
        .state
        .intelligence()
        .information_for_holder(KnowledgeHolder::Character(detective))
        .find(|fact| {
            matches!(
                fact.signal(),
                Some(InformationSignal::LegalPersonStatus(
                    LegalPersonStatusSignal::CaseWitness { .. }
                ))
            )
        })
        .expect("the police detective's actual case must register a first-hand witness fact");
    let private_source = private.id();
    let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
        investigation,
    })) = private.signal()
    else {
        panic!("expected a typed witness observation");
    };
    let case_target = format!("investigation-{}", investigation.raw());
    let investigation = *investigation;
    let witness = match private.subject() {
        EntityRef::Character(id) => id,
        _ => panic!("a typed witness source must name a character"),
    };
    assert!(
        !session.known_character_ids().contains(&witness),
        "detective notes must not identify a witness to leadership automatically"
    );
    assert!(
        session
            .execute(&format!("intel {}", private_source.raw()))
            .is_err(),
        "guessing the detective's private information ID must not reveal it"
    );
    assert!(
        session
            .execute(&format!("plan witnesspressure char-{}", witness.raw()))
            .is_err()
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("preview surveillance {case_target}"))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    assert!(session.execute("contact").is_err());
    assert_eq!(before, snapshot(&registry, &session));
    let learned = session
        .execute(&format!("contact {} ask", contact_id.raw()))
        .unwrap()
        .0;
    assert!(learned.contains("Case witness identified"), "{learned}");
    assert_ne!(before, snapshot(&registry, &session));
    let disclosed = session
        .organization_information()
        .find(|fact| {
            matches!(
                fact.signal(),
                Some(InformationSignal::LegalPersonStatus(
                    LegalPersonStatusSignal::CaseWitness { .. }
                ))
            )
        })
        .expect("contact disclosure must retain the typed legal-episode basis");
    assert_eq!(disclosed.subject(), EntityRef::Character(witness));
    assert!(disclosed.derived_from().contains(&private_source));
    let detail = session
        .execute(&format!("intel {}", disclosed.id().raw()))
        .unwrap()
        .0;
    assert!(
        detail.contains("Typed finding: LegalPersonStatus(CaseWitness"),
        "{detail}"
    );
    assert!(
        session
            .execute("known-people")
            .unwrap()
            .0
            .contains("reported case witness")
    );
    assert!(
        session
            .execute("known-targets")
            .unwrap()
            .0
            .contains(&case_target)
    );
    assert!(
        session
            .organization_information()
            .all(|fact| { fact.subject() != EntityRef::Investigation(investigation) }),
        "the witness disclosure alone should identify the case"
    );
    let before_preview = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("preview surveillance {case_target}"))
            .is_ok(),
        "a disclosed typed case identity must permit reconnaissance"
    );
    assert_eq!(before_preview, snapshot(&registry, &session));
    let pressure = session
        .execute(&format!("plan witnesspressure char-{}", witness.raw()))
        .unwrap()
        .0;
    assert!(
        pressure.contains("Authorized WitnessPressure"),
        "{pressure}"
    );
    let after = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        after,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
