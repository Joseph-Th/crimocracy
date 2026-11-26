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
fn recovered_acquired_storefront_never_leaks_previous_owners_cycle_books() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let independent = session
        .state
        .world()
        .public_business_directory()
        .find(|(id, _, _, _)| {
            session.state.world().get_business(*id).unwrap().owner() == BusinessOwner::Independent
        })
        .unwrap()
        .0;
    session.execute("advance 1d").unwrap();
    let previous = session
        .state
        .economy()
        .cycles_for(independent)
        .next_back()
        .expect("public independent storefront settles in the first day");
    assert_ne!(
        previous.owner(),
        BusinessOwner::Organization(session.player)
    );
    let accounted = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let settlement = fresh_settlement(&mut session);
    let settlement = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .find(|account| account.id().raw() == settlement)
        .unwrap()
        .id();
    let now = session.state.now();
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: now,
            memo: "Test acquisition funding".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: accounted,
                    amount: Money::from_cents(2_000_000),
                },
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-2_000_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    session
        .execute(&format!("buy {}", independent.raw()))
        .unwrap();
    session
        .execute(&format!("business {} suspend", independent.raw()))
        .unwrap();
    let before = snapshot(&registry, &session);
    let recovery = session.execute("recovery").unwrap().0;
    assert!(
        recovery.contains(&format!("business-{}", independent.raw())),
        "{recovery}"
    );
    assert!(
        !recovery.contains("Last settlement:"),
        "the latest independent-owner settlement is not the player's book: {recovery}"
    );
    assert_eq!(before, snapshot(&registry, &session));
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn manual_owner_draw_uses_receivable_treasury_and_reports_actual_receipt() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let owner = FinancialOwner::Organization(session.player);
    let indebted = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let clearing = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Test existing accounted obligation".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: indebted,
                    amount: Money::from_cents(-1_000),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(1_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    session.execute("open-account accounted").unwrap();
    let receiver = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::AccountedFunds && account.id() != indebted)
        .unwrap()
        .id();
    session.execute("advance 1d uninterrupted").unwrap();
    let business = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| {
            crimocracy::economy::business_economy_system::resolve_available_business_profit(
                &session.state,
                session.player,
                business.id(),
            )
            .is_ok_and(|amount| amount.cents() >= 1_000)
        })
        .unwrap()
        .id();
    let before = snapshot(&registry, &session);
    let output = session
        .execute(&format!("sweep {} 1000", business.raw()))
        .unwrap()
        .0;
    assert_eq!(
        session
            .state
            .finance()
            .get_account(indebted)
            .unwrap()
            .balance()
            .cents(),
        -1_000,
        "an ordinary owner distribution must not implicitly pay an unrelated debt"
    );
    assert_eq!(
        session
            .state
            .finance()
            .get_account(receiver)
            .unwrap()
            .balance()
            .cents(),
        1_000
    );
    assert!(output.contains("received 1000 cents"), "{output}");
    assert!(output.contains("administration 0 cents"), "{output}");
    let after = snapshot(&registry, &session);
    session.state = restore_save(&registry, bincode::deserialize(&before).unwrap()).unwrap();
    assert_eq!(
        session
            .execute(&format!("sweep {} 1000", business.raw()))
            .unwrap()
            .0,
        output
    );
    assert_eq!(snapshot(&registry, &session), after);
}

#[test]
fn explicit_max_and_reserved_laundering_orders_use_real_front_capacity() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| {
            remaining_laundering_capacity(&registry, &session.state, session.player, business.id())
                .is_some_and(|room| room.cents() > 0)
        })
        .unwrap()
        .id();
    let first_room =
        remaining_laundering_capacity(&registry, &session.state, session.player, front)
            .unwrap()
            .cents();
    let street = starting_cash(&session);
    let street_before = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .find(|account| account.id().raw() == street)
        .unwrap()
        .available_payment_balance()
        .cents();
    let before = snapshot(&registry, &session);
    for command in [
        "launder-all -1",
        "launder-all nope",
        "launder-all",
        "launder-all 0 extra",
    ] {
        assert!(session.execute(command).is_err(), "{command}");
        assert_eq!(before, snapshot(&registry, &session));
    }
    let reserve = street_before - 10_000;
    let first = session
        .execute(&format!("launder-all {reserve}"))
        .unwrap()
        .0;
    assert!(first.contains("10000 street cents converted"), "{first}");
    assert!(first.contains("transaction-"), "{first}");
    let street_after = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .find(|account| account.id().raw() == street)
        .unwrap()
        .available_payment_balance()
        .cents();
    assert_eq!(
        street_after, reserve,
        "batch must preserve the requested reserve"
    );
    let first_save = snapshot(&registry, &session);
    let repeated = session
        .execute(&format!("launder-all {reserve}"))
        .unwrap()
        .0;
    assert!(repeated.contains("0 street cents converted"), "{repeated}");
    assert_eq!(first_save, snapshot(&registry, &session));

    let additional = session.execute(&format!("launder {} max", front.raw()));
    assert!(additional.is_ok(), "{additional:?}");
    let remaining = remaining_laundering_capacity(&registry, &session.state, session.player, front)
        .unwrap()
        .cents();
    assert!(remaining < first_room);
    let no_room = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("launder {} max", front.raw()))
            .is_err()
    );
    assert_eq!(no_room, snapshot(&registry, &session));
    assert!(
        session
            .execute("launder-all 0")
            .unwrap()
            .0
            .contains("Laundering order complete")
    );
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn laundering_orders_pool_split_street_balances_without_extra_transactions() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let owner = FinancialOwner::Organization(session.player);
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| {
            remaining_laundering_capacity(&registry, &session.state, session.player, business.id())
                .is_some_and(|room| room.cents() >= 10_000)
        })
        .unwrap()
        .id();
    let source = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::StreetCash)
        .unwrap();
    let street = source.id();
    let surplus = source.balance().cents() - 10_000;
    let second = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Split test treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: street,
                    amount: Money::from_cents(-surplus - 4_000),
                },
                LedgerPosting {
                    account: second,
                    amount: Money::from_cents(4_000),
                },
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(surplus),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let opening = build_save(&registry, &session.state).unwrap();
    for (command, expected, reserve) in [
        (format!("launder {} max", front.raw()), 10_000, 0),
        ("launder-all 2000".to_owned(), 8_000, 2_000),
    ] {
        session.state = restore_save(&registry, opening.clone()).unwrap();
        let before_count = session
            .state
            .finance()
            .recent_transactions_for_owner(owner, 100)
            .len();
        let result = session.execute(&command).unwrap().0;
        assert!(result.contains(&expected.to_string()), "{result}");
        assert_eq!(
            session
                .state
                .finance()
                .recent_transactions_for_owner(owner, 100)
                .len(),
            before_count + 1
        );
        let remaining: i64 = session
            .state
            .finance()
            .accounts_for(owner)
            .filter(|account| account.kind() == AccountKind::StreetCash)
            .map(|account| account.available_payment_balance().cents())
            .sum();
        assert_eq!(remaining, reserve, "{result}");
        let saved = build_save(&registry, &session.state).unwrap();
        let restored = restore_save(&registry, saved.clone()).unwrap();
        assert_eq!(
            bincode::serialize(&saved).unwrap(),
            bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
        );
    }
}

#[test]
fn resume_rackets_batch_prevalidates_every_selected_lifecycle_change() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let original = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap();
    let first = original.id();
    let manager = original.manager();
    let district = session.state.world().neighborhoods().next().unwrap().id();
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    session
        .execute(&format!(
            "start-racket protection district-{} {cash} {settlement} manager=char-{}",
            district.raw(),
            manager.raw()
        ))
        .unwrap();
    let second = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|enterprise| enterprise.id() != first)
        .unwrap()
        .id();

    session
        .execute(&format!("racket {} suspend", first.raw()))
        .unwrap();
    let before = snapshot(&registry, &session);
    for command in [
        format!("resume-rackets {} {}", first.raw(), first.raw()),
        format!("resume-rackets {} 999999", first.raw()),
        format!("resume-rackets {} {}", first.raw(), second.raw()),
    ] {
        assert!(session.execute(&command).is_err(), "{command}");
        assert_eq!(before, snapshot(&registry, &session));
    }
    session
        .execute(&format!("racket {} suspend", second.raw()))
        .unwrap();
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("resume-rackets {} 999999", first.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    let response = session
        .execute(&format!("resume-rackets {} {}", second.raw(), first.raw()))
        .unwrap()
        .0;
    assert!(
        response.contains("Resumed 2 selected racket(s)"),
        "{response}"
    );
    for id in [first, second] {
        assert_eq!(
            session
                .state
                .enterprises()
                .get_enterprise(id)
                .unwrap()
                .status(),
            EnterpriseStatus::Active
        );
    }
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn reassign_racket_preserves_cycle_history_and_rejects_unmandated_members() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let record = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap();
    let id = record.id();
    let former = record.manager();
    let cash = record.cash_account();
    let settlement = record.settlement_account();
    let successor = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| {
            member.id() != former
                && member.supervisor().is_some()
                && session
                    .state
                    .delegation()
                    .active_for_manager(member.id())
                    .is_none()
        })
        .unwrap()
        .id();
    let command = format!("reassign-racket {} {}", id.raw(), successor.raw());
    let before = snapshot(&registry, &session);
    assert!(session.execute(&command).is_err());
    assert_eq!(snapshot(&registry, &session), before);
    session
        .execute(&format!("delegate {} enterprise", successor.raw()))
        .unwrap();
    session.execute("advance 1d").unwrap();
    let old = session.state.enterprises().latest_cycle(id).unwrap().id();
    session
        .execute(&format!("racket {} suspend", id.raw()))
        .unwrap();
    assert!(
        session
            .execute(&command)
            .unwrap()
            .0
            .contains("Still suspended")
    );
    session
        .execute(&format!("racket {} resume", id.raw()))
        .unwrap();
    let record = session.state.enterprises().get_enterprise(id).unwrap();
    assert_eq!(record.manager(), successor);
    assert_eq!(record.cash_account(), cash);
    assert_eq!(record.settlement_account(), settlement);
    assert_eq!(
        session.state.enterprises().latest_cycle(id).unwrap().id(),
        old
    );
    assert!(
        session
            .execute(&format!("racket-cycles {}", id.raw()))
            .unwrap()
            .0
            .contains(&format!("manager char-{}", former.raw()))
    );
    let _ = snapshot(&registry, &session);
}

#[test]
fn player_sees_management_concentration_before_and_after_establishment() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let original = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap();
    let manager = original.manager();
    let district = session.state.world().neighborhoods().next().unwrap().id();
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    let args = format!(
        "protection district-{} {cash} {settlement} manager=char-{}",
        district.raw(),
        manager.raw()
    );
    let before = snapshot(&registry, &session);
    assert!(
        !session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Management concentration")
    );
    assert!(
        session
            .execute(&format!("preview-racket {args}"))
            .unwrap()
            .0
            .contains("2 active rackets")
    );
    assert_eq!(before, snapshot(&registry, &session));
    session.execute(&format!("start-racket {args}")).unwrap();
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Detention would suspend all of them")
    );
    assert_eq!(before, snapshot(&registry, &session));
    let second = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|r| r.kind() == EnterpriseKind::Protection)
        .unwrap()
        .id();
    session
        .execute(&format!("racket {} suspend", second.raw()))
        .unwrap();
    assert!(
        !session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Management concentration")
    );
}

#[test]
fn racket_outlook_is_owned_read_only_and_distinguishes_unknown_from_observed_heat() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let id = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    let rival = session
        .state
        .world()
        .public_business_directory()
        .find_map(|(business, _, _, _)| {
            session
                .state
                .enterprises()
                .enterprises_at(EnterpriseLocation::Business(business))
                .find(|e| e.organization() != session.player)
        })
        .unwrap()
        .id();
    let command = format!("racket-outlook {}", id.raw());
    let before = snapshot(&registry, &session);
    let first = session.execute(&command).unwrap().0;
    assert!(first.contains("Own-operation pressure unknown"));
    assert!(first.contains("spendable cash"));
    assert!(
        session
            .execute(&format!("racket-outlook {}", rival.raw()))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    session.execute("advance 1d").unwrap();
    let after = snapshot(&registry, &session);
    let second = session.execute(&command).unwrap().0;
    assert!(
        second.contains("Last observed own-operation pressure in district"),
        "{second}"
    );
    assert_eq!(after, snapshot(&registry, &session));
}

#[test]
fn district_racket_uses_the_assigned_manager_and_not_the_general_mandate() {
    use crimocracy::world::CapabilityKind;
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| {
            member
                .capability(CapabilityKind::Surveillance)
                .is_some_and(|rating| rating.value() == 91)
        })
        .unwrap()
        .id();
    let district = session.state.world().neighborhoods().last().unwrap().id();
    session
        .execute(&format!(
            "delegate char-{} district-{}",
            manager.raw(),
            district.raw()
        ))
        .unwrap();
    let location = EnterpriseLocation::Neighborhood(district);
    let authority = session
        .enterprise_authority(location, &BTreeSet::new(), None)
        .unwrap();
    assert_eq!(authority.manager, manager);
    assert_eq!(authority.scope, ResponsibilityScope::Neighborhood(district));
    let outside = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .next()
        .unwrap()
        .id();
    let general = session
        .enterprise_authority(location, &BTreeSet::from([outside]), None)
        .unwrap();
    assert_eq!(
        general.scope,
        ResponsibilityScope::Function(ResponsibilityFunction::Enterprise)
    );
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    session
        .execute(&format!(
            "start-racket protection district-{} {} {}",
            district.raw(),
            cash,
            settlement
        ))
        .unwrap();
    let record = session
        .state
        .enterprises()
        .enterprises_at(location)
        .find(|enterprise| enterprise.organization() == session.player)
        .unwrap();
    assert_eq!(record.authority().manager, manager);
}

#[test]
fn enterprise_preview_and_explicit_manager_share_establishment_validation() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| {
            person
                .capability(crimocracy::world::CapabilityKind::Surveillance)
                .is_some_and(|rating| rating.value() == 91)
        })
        .unwrap()
        .id();
    let district = session.state.world().neighborhoods().last().unwrap().id();
    session
        .execute(&format!(
            "delegate char-{} district-{}",
            manager.raw(),
            district.raw()
        ))
        .unwrap();
    let cash = starting_cash(&session);
    let settlement = fresh_settlement(&mut session);
    let support = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .next()
        .unwrap()
        .id();
    let bookkeeper = insert_business(
        &registry,
        &mut session.state,
        BusinessDraft {
            name: "District Accounting".to_owned(),
            kind: BusinessKind::ProfessionalServices,
            functions: BTreeSet::from([BusinessFunction::ProfessionalRecords]),
            neighborhood: district,
            owner: BusinessOwner::Organization(session.player),
        },
    )
    .unwrap();
    let args = format!(
        "numbersracket district-{} {cash} {settlement} business-{} business-{} manager=char-{}",
        district.raw(),
        support.raw(),
        bookkeeper.raw(),
        manager.raw()
    );
    let before = snapshot(&registry, &session);
    for verb in ["preview-racket", "start-racket"] {
        assert!(
            session
                .execute(&format!("{verb} {args}"))
                .unwrap_err()
                .contains("entire supporting network")
        );
        assert_eq!(before, snapshot(&registry, &session));
    }
    let mandate = session
        .state
        .delegation()
        .active_for_manager(manager)
        .unwrap()
        .id();
    session
        .execute(&format!("mandate {} add enterprise", mandate.raw()))
        .unwrap();
    let before = snapshot(&registry, &session);
    let preview = session
        .execute(&format!("preview-racket {args}"))
        .unwrap()
        .0;
    assert!(preview.contains("Preview only"));
    assert!(preview.contains(&format!("char-{}", manager.raw())));
    assert!(preview.contains("Conditional operating cost"));
    assert!(preview.contains("New police inquiry risk per operating cycle"));
    assert!(preview.contains("Suspending stops new racket-generated inquiry rolls"));
    assert_eq!(before, snapshot(&registry, &session));
    for suffix in [
        format!("manager=char-{}", manager.raw()),
        "manager=char-999999".to_owned(),
    ] {
        assert!(
            session
                .execute(&format!("start-racket {args} {suffix}"))
                .is_err()
        );
        assert_eq!(before, snapshot(&registry, &session));
    }
    let started = session.execute(&format!("start-racket {args}")).unwrap().0;
    assert!(started.contains(&format!("char-{}", manager.raw())));
    let record = session
        .state
        .enterprises()
        .enterprises_at(EnterpriseLocation::Neighborhood(district))
        .find(|record| {
            record.organization() == session.player
                && record.kind() == EnterpriseKind::NumbersRacket
        })
        .unwrap();
    assert_eq!(record.manager(), manager);
    assert_eq!(
        record.authority().scope,
        ResponsibilityScope::Function(ResponsibilityFunction::Enterprise)
    );
    let outlook = session
        .execute(&format!("racket-outlook {}", record.id().raw()))
        .unwrap()
        .0;
    for line in preview.lines().filter(|line| {
        line.contains("Conditional operating cost") || line.contains("Expected cycle net")
    }) {
        assert!(outlook.contains(line), "{outlook}");
    }
}

#[test]
fn owned_storefront_observations_do_not_appear_as_business_leads() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let owned = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .next()
        .unwrap()
        .id();
    session
        .execute(&format!("scout business-{}", owned.raw()))
        .unwrap();
    session.execute("advance 3h").unwrap();
    let view = session.execute("businesses").unwrap().0;
    let leads = view.split("Known business leads:\n").nth(1).unwrap();
    assert!(
        !leads
            .lines()
            .any(|line| line.starts_with(&format!("{} ", owned.raw())))
    );
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
    let recovery = session.execute("recovery").unwrap().0;
    assert!(
        recovery.contains("Suspended storefront recovery"),
        "{recovery}"
    );
    assert!(
        recovery.contains(&format!(
            "Can resume manually: business {} resume",
            front.raw()
        )),
        "{recovery}"
    );
    assert!(
        session
            .execute("brief")
            .unwrap()
            .0
            .contains("suspended storefronts: 1")
    );
    assert_eq!(stopped, snapshot(&registry, &session));
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
    assert!(!session.execute("recovery").unwrap().0.contains(&format!(
        "Can resume manually: business {} resume",
        front.raw()
    )));
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
        district: session.district.clone(),
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
