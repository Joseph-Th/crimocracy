//! Player economy and district enterprise commands across canonical owners.

use super::*;
use crimocracy::enterprises::EnterpriseStatus;
use crimocracy::world::world_system::insert_business;
use crimocracy::world::{BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner};

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

fn fresh_settlement(session: &mut Session<'_>) -> u32 {
    let before: BTreeSet<_> = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .map(|account| account.id())
        .collect();
    session.execute("open-account settlement").unwrap();
    session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .filter(|account| {
            account.kind() == AccountKind::Settlement && !before.contains(&account.id())
        })
        .map(|account| account.id().raw())
        .next()
        .unwrap()
}

fn starting_cash(session: &Session<'_>) -> u32 {
    session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .filter(|account| account.kind() == AccountKind::StreetCash)
        .max_by_key(|account| account.balance().cents())
        .unwrap()
        .id()
        .raw()
}

#[test]
fn text_play_owned_business_can_pause_and_resume_without_laundering_reset() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| business.name() == "Fulton Social Club")
        .unwrap()
        .id();
    let original = snapshot(&registry, &session);
    for command in [
        "business business-1 suspend",
        "business business-4 suspend",
        "business business-3 resume",
        "business business-3 retire",
        "business business-3 suspend ignored",
        "business business-99 suspend",
        "business",
    ] {
        assert!(session.execute(command).is_err(), "{command} must reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} mutated campaign state on rejection"
        );
    }
    session
        .execute(&format!("launder {} 5000", front.raw()))
        .unwrap();
    let before =
        remaining_laundering_capacity(&registry, &session.state, session.player, front).unwrap();
    let paid = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("business {} suspend", front.raw()))
            .is_err(),
        "a front serving a live racket is unavailable for manual shutdown"
    );
    assert_eq!(paid, snapshot(&registry, &session));
    let racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    session
        .execute(&format!("racket {} suspend", racket.raw()))
        .unwrap();
    assert!(
        session
            .execute(&format!("business {} suspend", front.raw()))
            .unwrap()
            .0
            .contains("Suspended")
    );
    assert!(
        session
            .execute("businesses")
            .unwrap()
            .0
            .contains("Suspended")
    );
    let stopped = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("launder {} 1000", front.raw()))
            .is_err()
    );
    assert!(
        session
            .execute(&format!("racket {} resume", racket.raw()))
            .is_err()
    );
    assert_eq!(stopped, snapshot(&registry, &session));
    assert!(
        session
            .execute(&format!("business {} resume", front.raw()))
            .unwrap()
            .0
            .contains("Active")
    );
    assert_eq!(
        before,
        remaining_laundering_capacity(&registry, &session.state, session.player, front).unwrap(),
        "resuming a storefront must not reset its laundering budget"
    );
    session
        .execute(&format!("racket {} resume", racket.raw()))
        .unwrap();
    let resumed = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        resumed,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn text_play_district_racket_settles_and_restores_with_original_business_racket() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    let before = snapshot(&registry, &session);
    for command in [
        format!("start-racket gambling district-1 {cash} {settlement}"),
        format!("start-racket protection district-99 {cash} {settlement}"),
        format!("start-racket protection district-1 {cash} account-10"),
        format!("start-racket protection district-1 {cash} {settlement} business-1"),
        format!("start-racket protection 1 {cash} {settlement}"),
        format!("start-racket protection district-1 {cash} {settlement} 3 3"),
    ] {
        assert!(session.execute(&command).is_err(), "{command} must reject");
        assert_eq!(
            before,
            snapshot(&registry, &session),
            "{command} mutated state on rejection"
        );
    }
    let confirmation = session
        .execute(&format!(
            "start-racket protection district-1 {cash} {settlement}"
        ))
        .unwrap()
        .0;
    assert!(
        confirmation.contains("district-1 South Ward"),
        "{confirmation}"
    );
    let racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|record| record.kind() == EnterpriseKind::Protection)
        .unwrap();
    assert!(matches!(
        racket.location(),
        EnterpriseLocation::Neighborhood(_)
    ));
    assert_eq!(racket.cash_account().raw(), cash);
    assert_eq!(racket.settlement_account().raw(), settlement);
    let racket_id = racket.id();
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("district-1 South Ward")
    );
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains(&format!("settlement account-{settlement}"))
    );
    let active = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!(
                "start-racket protection district-1 {cash} {settlement}"
            ))
            .is_err()
    );
    assert_eq!(active, snapshot(&registry, &session));
    let advanced = session.execute("advance 1d").unwrap().0;
    assert!(advanced.contains("Advanced 1440 minute"), "{advanced}");
    let cycle = session
        .state
        .enterprises()
        .latest_cycle(racket_id)
        .expect("district racket must settle through the canonical cycle");
    assert!(cycle.gross_revenue() > Money::ZERO);
    assert!(
        session
            .state
            .enterprises()
            .latest_cycle(
                session
                    .state
                    .enterprises()
                    .enterprises_for_organization(session.player)
                    .find(|record| record.kind() == EnterpriseKind::Gambling)
                    .unwrap()
                    .id()
            )
            .is_some(),
        "the original business-based racket must continue settling"
    );
    let after = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        after,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn text_play_district_racket_can_require_two_distinct_owned_supports() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let district = session
        .state
        .world()
        .neighborhoods()
        .find(|record| record.name() == "South Ward")
        .unwrap()
        .id();
    // Add a legitimate bookkeeper to the test fixture through world creation,
    // not through private-record mutation. The playable purchase route also
    // offers independent firms through the public business directory.
    let bookkeeper = insert_business(
        &registry,
        &mut session.state,
        BusinessDraft {
            name: "South Ward Accounting".to_owned(),
            kind: BusinessKind::ProfessionalServices,
            functions: BTreeSet::from([BusinessFunction::ProfessionalRecords]),
            neighborhood: district,
            owner: BusinessOwner::Organization(session.player),
        },
    )
    .unwrap();
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| business.name() == "Fulton Social Club")
        .unwrap()
        .id();
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    let command = format!(
        "start-racket numbersracket district-{} {cash} {settlement} business-{} business-{}",
        district.raw(),
        front.raw(),
        bookkeeper.raw()
    );
    let before = snapshot(&registry, &session);
    for invalid in [
        format!(
            "start-racket numbersracket district-{} {cash} {settlement}",
            district.raw()
        ),
        format!(
            "start-racket numbersracket district-{} {cash} {settlement} {}",
            district.raw(),
            front.raw()
        ),
        format!(
            "start-racket numbersracket district-{} {cash} {settlement} {} {}",
            district.raw(),
            front.raw(),
            front.raw()
        ),
        format!(
            "start-racket numbersracket district-{} {cash} {settlement} {} 1",
            district.raw(),
            front.raw()
        ),
    ] {
        assert!(session.execute(&invalid).is_err(), "{invalid} must reject");
        assert_eq!(before, snapshot(&registry, &session));
    }
    let created = session.execute(&command).unwrap().0;
    assert!(
        created.contains("using 2 supporting business(es)"),
        "{created}"
    );
    let racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|record| record.kind() == EnterpriseKind::NumbersRacket)
        .unwrap();
    assert_eq!(
        racket.supporting_businesses(),
        &BTreeSet::from([front, bookkeeper])
    );
    assert_eq!(racket.status(), EnterpriseStatus::Active);
    let id = racket.id();
    let display = session.execute("enterprises").unwrap().0;
    assert!(display.contains(&format!("business-{}", bookkeeper.raw())));
    assert!(
        session
            .execute(&format!("business {} suspend", front.raw()))
            .is_err(),
        "business infrastructure in a live district racket cannot be suspended"
    );
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
    session
        .execute(&format!("racket {} suspend", id.raw()))
        .unwrap();
    assert!(
        session
            .execute(&format!("business {} suspend", front.raw()))
            .is_err(),
        "the existing gambling racket still needs its venue"
    );
    session
        .execute(&format!("racket {} resume", id.raw()))
        .unwrap();
}

#[test]
fn text_play_ledger_and_asset_cycle_views_explain_multi_day_performance() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| business.name() == "Fulton Social Club")
        .unwrap()
        .id();
    let original = snapshot(&registry, &session);
    for command in [
        "business-cycles business-1",
        "business-cycles business-3 0",
        "business-cycles business-3 8 extra",
        "business-cycles char-3",
        "racket-cycles enterprise-99",
        "racket-cycles business-1",
        "racket-cycles enterprise-1 invalid",
    ] {
        assert!(session.execute(command).is_err(), "{command} should reject");
        assert_eq!(original, snapshot(&registry, &session));
    }
    let empty = session
        .execute(&format!("business-cycles {} 2", front.raw()))
        .unwrap()
        .0;
    assert!(empty.contains("No settled operating cycles"), "{empty}");
    let empty = session.execute("racket-cycles enterprise-1").unwrap().0;
    assert!(empty.contains("No settled cycles"), "{empty}");

    let settlement = fresh_settlement(&mut session);
    let cash = starting_cash(&session);
    session
        .execute(&format!(
            "start-racket protection district-1 {cash} {settlement}"
        ))
        .unwrap();
    let district_racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|record| record.kind() == EnterpriseKind::Protection)
        .unwrap()
        .id();
    // Run the canonical tick to capture the complete published accounting
    // chronology without the terminal client's optional attention pauses.
    for _ in 0..(DAY_MINUTES * 2) {
        run_tick(&registry, &mut session.state).unwrap();
    }
    let business = session
        .execute(&format!("business-cycles business-{} 2", front.raw()))
        .unwrap()
        .0;
    assert!(business.contains("business-cycle-"), "{business}");
    assert!(business.contains("gross ") && business.contains("cost ") && business.contains("net "));
    assert_eq!(business.matches("business-cycle-").count(), 2);
    let original_racket = session.execute("racket-cycles enterprise-1 2").unwrap().0;
    assert_eq!(original_racket.matches("enterprise-cycle-").count(), 2);
    assert!(original_racket.contains("heat ") && original_racket.contains("enforcement:"));
    let district = session
        .execute(&format!("racket-cycles {} 2", district_racket.raw()))
        .unwrap()
        .0;
    assert!(district.contains("district-1 South Ward"));
    assert_eq!(district.matches("enterprise-cycle-").count(), 2);
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut resumed = Session {
        registry: &registry,
        state: restored,
        player: session.player,
        seeds: session.seeds,
        district: session.district,
        district_variant: session.district_variant,
    };
    assert_eq!(
        business,
        resumed
            .execute(&format!("business-cycles {} 2", front.raw()))
            .unwrap()
            .0
    );
    assert_eq!(
        original_racket,
        resumed.execute("racket-cycles enterprise-1 2").unwrap().0
    );
    assert_eq!(
        district,
        resumed
            .execute(&format!("racket-cycles {} 2", district_racket.raw()))
            .unwrap()
            .0
    );
    assert_eq!(saved, snapshot(&registry, &resumed));
}
