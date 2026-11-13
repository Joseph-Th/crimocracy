//! Personal observations become leadership knowledge only through an actual
//! transfer record. An arbitrary character or information ID cannot disclose
//! a police officer's, rival's, or detainee's private knowledge.

use super::*;
use crimocracy::intelligence::intelligence_system::validate_record_information;
use crimocracy::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
};

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

fn member(session: &Session<'_>, name: &str) -> CharacterId {
    session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|character| character.name() == name)
        .unwrap()
        .id()
}

#[test]
fn debrief_only_permits_current_members_and_is_read_only_when_empty() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let expected = snapshot(&registry, &session);
    let view = session.execute("debriefs").unwrap().0;
    assert!(view.contains("Carlo Venn | 0 unreported observation(s)"));
    assert!(!view.contains("Harlan Pike"));
    assert!(
        session
            .execute("debrief char-3")
            .unwrap()
            .0
            .contains("no new observations")
    );
    for command in [
        "debrief char-1",
        "debrief 1",
        "debrief 999",
        "debrief business-3",
        "debrief 3 extra",
        "debriefs extra",
    ] {
        assert!(session.execute(command).is_err(), "{command} should reject");
        assert_eq!(
            expected,
            snapshot(&registry, &session),
            "{command} changed authoritative state"
        );
    }
    assert_eq!(expected, snapshot(&registry, &session));
}

#[test]
fn debrief_preserves_personal_source_lineage_and_is_idempotent_on_repeat() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let scout = member(&session, "Mara Vale");
    let subject = member(&session, "Carlo Venn");
    let original = validate_record_information(
        &session.state,
        InformationDraft {
            holder: KnowledgeHolder::Character(scout),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Character(subject),
            observed_at: session.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "Mara saw Carlo meeting the ward organizer in person.".to_owned(),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    assert!(
        !session
            .organization_information()
            .any(|fact| fact.id() == original)
    );
    assert!(
        !session
            .execute("intelligence")
            .unwrap()
            .0
            .contains("meeting the ward organizer")
    );
    assert!(
        session
            .execute(&format!("intel {}", original.raw()))
            .is_err(),
        "a member's personal note is not yet organization-held information"
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute("debriefs")
            .unwrap()
            .0
            .contains("Mara Vale | 1 unreported observation(s)")
    );
    assert_eq!(
        before,
        snapshot(&registry, &session),
        "a debrief directory must be read-only"
    );
    let receipt = session
        .execute(&format!("debrief {}", scout.raw()))
        .unwrap()
        .0;
    assert!(
        receipt.contains("Mara saw Carlo meeting the ward organizer"),
        "{receipt}"
    );
    let shared = session
        .organization_information()
        .find(|fact| {
            fact.source_kind() == InformationSourceKind::InternalReport
                && fact.derived_from().contains(&original)
        })
        .expect("source must be transferred through the production intelligence owner");
    assert_eq!(
        shared.holder(),
        KnowledgeHolder::Organization(session.player)
    );
    assert_eq!(shared.subject(), EntityRef::Character(subject));
    assert_eq!(shared.source_entity(), Some(EntityRef::Character(scout)));
    assert_eq!(shared.reliability(), Reliability::DirectAccess);
    assert_eq!(shared.specificity(), Specificity::Precise);
    assert_eq!(shared.observed_at(), session.state.now());
    assert_eq!(shared.derived_from(), &BTreeSet::from([original]));
    let detail = session
        .execute(&format!("intel {}", shared.id().raw()))
        .unwrap()
        .0;
    assert!(detail.contains("InternalReport"), "{detail}");
    assert!(detail.contains(&format!("information-{}", original.raw())));
    assert!(detail.contains("Mara saw Carlo meeting the ward organizer"));
    assert!(
        session
            .execute(&format!("intel {}", original.raw()))
            .is_err(),
        "the source note itself must remain with its original holder"
    );
    let after = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("debrief {}", scout.raw()))
            .unwrap()
            .0
            .contains("no new observations")
    );
    assert_eq!(
        after,
        snapshot(&registry, &session),
        "a second debrief is not a new source"
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut resumed = Session {
        registry: &registry,
        state: restored,
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert_eq!(after, snapshot(&registry, &resumed));
    assert!(
        resumed
            .execute(&format!("debrief {}", scout.raw()))
            .unwrap()
            .0
            .contains("no new observations")
    );
}

#[test]
fn real_police_response_creates_member_notes_that_require_separate_debriefs() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    let report = session.execute("advance 1d").unwrap().0;
    assert!(
        report.contains("operation reached a terminal outcome"),
        "{report}"
    );
    let boss = member(&session, "Joseph Marrow");
    let lieutenant = member(&session, "Carlo Venn");
    let observations = session
        .state
        .intelligence()
        .information_for_holder(KnowledgeHolder::Character(lieutenant))
        .filter(|fact| fact.topic() == InformationTopic::PoliceActivity)
        .count();
    assert!(
        observations > 0,
        "canonical police response actually gave the crew first-hand pressure knowledge"
    );
    let before = snapshot(&registry, &session);
    assert!(
        !session
            .execute("intelligence")
            .unwrap()
            .0
            .contains("directly experienced Central Precinct responding")
    );
    assert!(
        session
            .execute("debriefs")
            .unwrap()
            .0
            .contains("Carlo Venn | 2 unreported observation(s)")
    );
    assert_eq!(before, snapshot(&registry, &session));
    let receipt = session
        .execute(&format!("debrief {}", lieutenant.raw()))
        .unwrap()
        .0;
    assert!(
        receipt.contains("directly experienced Central Precinct responding"),
        "{receipt}"
    );
    let first = session
        .organization_information()
        .filter(|fact| fact.source_kind() == InformationSourceKind::InternalReport)
        .count();
    assert_eq!(first, 1);
    session
        .execute(&format!("debrief {}", lieutenant.raw()))
        .unwrap();
    session.execute(&format!("debrief {}", boss.raw())).unwrap();
    session.execute(&format!("debrief {}", boss.raw())).unwrap();
    let shared = session
        .organization_information()
        .filter(|fact| fact.source_kind() == InformationSourceKind::InternalReport)
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 4);
    assert!(
        shared
            .iter()
            .any(|fact| fact.topic() == InformationTopic::PoliceActivity)
    );
    assert!(
        shared
            .iter()
            .any(|fact| fact.topic() == InformationTopic::OperationalOutcome)
    );
    assert!(
        session
            .execute("debriefs")
            .unwrap()
            .0
            .contains("Carlo Venn | 0 unreported observation(s)")
    );
    assert!(
        session
            .execute("debriefs")
            .unwrap()
            .0
            .contains("Joseph Marrow | 0 unreported observation(s)")
    );
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn detained_member_cannot_be_debriefed_and_rejected_order_is_inert() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    for _ in 0..(DAY_MINUTES * 2) {
        if !session.known_member_arrests().is_empty() {
            break;
        }
        run_tick(&registry, &mut session.state).unwrap();
    }
    assert!(!session.known_member_arrests().is_empty());
    let detained = member(&session, "Joseph Marrow");
    assert!(
        session
            .state
            .legal()
            .active_arrest_for_character(detained)
            .is_some()
    );
    let before = snapshot(&registry, &session);
    let view = session.execute("debriefs").unwrap().0;
    assert!(
        view.contains("Joseph Marrow | in custody, unavailable for debrief"),
        "{view}"
    );
    assert!(
        session
            .execute(&format!("debrief {}", detained.raw()))
            .unwrap_err()
            .contains("detained member is unavailable")
    );
    assert_eq!(before, snapshot(&registry, &session));
}
