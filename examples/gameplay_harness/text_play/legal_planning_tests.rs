//! Person-targeting must depend on acquired information, not a raw character ID.

use super::*;
use crimocracy::intelligence::intelligence_system::validate_record_information;
use crimocracy::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
};
use crimocracy::world::BusinessOwner;

#[test]
fn text_play_knowing_a_person_does_not_invent_an_actionable_witness_or_arrest() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let owner = match session
        .state
        .world()
        .get_business(session.known_business(1).unwrap())
        .unwrap()
        .owner()
    {
        BusinessOwner::Character(character) => character,
        _ => panic!("authored primary target must have a personal owner"),
    };
    let raw = owner.raw();
    let initial = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        !session
            .execute("known-people")
            .unwrap()
            .0
            .contains(&format!("char-{raw} "))
    );
    assert!(
        session
            .execute(&format!("plan witnesspressure char-{raw}"))
            .unwrap_err()
            .contains("not an organization member or identified")
    );
    assert!(
        session
            .execute(&format!("plan extraction char-{raw}"))
            .is_err()
    );
    assert!(
        session
            .execute(&format!("plan witnesspressure {raw}"))
            .is_err()
    );
    assert_eq!(
        initial,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );

    // A canonical direct observation identifies the owner; it does not contain
    // the episode-bound legal-person status required for either legal plan.
    let observation = validate_record_information(
        &session.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(session.player),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Character(owner),
            observed_at: session.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "Identified owner in person at the shop.".to_owned(),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let directory = session.execute("known-people").unwrap().0;
    assert!(directory.contains(&format!("char-{raw} ")));
    assert!(!directory.contains("reported detention"));
    assert!(!directory.contains("reported case witness"));
    assert!(
        session
            .state
            .intelligence()
            .get_information(observation)
            .is_some()
    );
    let known = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let pressure = session
        .execute(&format!("plan witnesspressure char-{raw}"))
        .unwrap_err();
    assert!(pressure.contains("learned legal-status basis"));
    let extraction = session
        .execute(&format!("plan extraction char-{raw}"))
        .unwrap_err();
    assert!(extraction.contains("learned legal-status basis"));
    assert_eq!(
        known,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        known,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
