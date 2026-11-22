use crate::build_registry;
use crate::city_generation::{CityGenerationConfig, CityScale, generate_city};
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::recruitment::recruitment_system::validate_recruitment_attempt;
use crate::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crate::world::freelance_contracts::{
    FreelanceError, apply_due_freelance_expirations, validate_hire_freelancer,
};
use crate::world::{CivicProfession, IndependentVocation};

#[test]
fn freelance_career_change_is_blocked_during_a_contract_but_valid_after_expiry() {
    use crate::world::character_life::validate_set_civic_occupation;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let freelancer = city
        .state
        .world()
        .characters()
        .find(|person| {
            person.life().vocation() == IndependentVocation::FreelanceCriminal
                && city
                    .state
                    .social()
                    .get_relationship(person.id(), city.boss)
                    .is_some()
        })
        .unwrap()
        .id();
    let home = city
        .state
        .world()
        .get_character(freelancer)
        .unwrap()
        .life()
        .home()
        .unwrap();
    validate_hire_freelancer(&city.state, city.player, freelancer, city.boss, 1)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(matches!(
        validate_set_civic_occupation(
            &city.state,
            freelancer,
            home,
            IndependentVocation::Resident,
        ),
        Err(crate::world::character_life::CivicPersonError::ActiveFreelanceContract(id))
            if id == freelancer
    ));
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );

    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    assert_eq!(
        apply_due_freelance_expirations(&mut city.state),
        vec![freelancer]
    );
    validate_set_civic_occupation(
        &city.state,
        freelancer,
        home,
        IndependentVocation::Journalist,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert!(
        city.state
            .world()
            .active_freelance_contract(freelancer)
            .is_none()
    );
    assert!(
        city.state
            .world()
            .freelance_contracts()
            .any(|contract| contract.freelancer() == freelancer && contract.ended_at().is_some())
    );
    validate_invariants(&city.state);
    let saved = build_save(&registry, &city.state).unwrap();
    let restored = restore_save(&registry, saved).unwrap();
    assert_eq!(
        restored
            .world()
            .get_character(freelancer)
            .unwrap()
            .life()
            .vocation(),
        IndependentVocation::Journalist
    );
}

#[test]
fn city_scales_create_hundreds_of_distinct_live_people_with_actual_professions() {
    let registry = build_registry();
    let targets = [
        (CityScale::Small.config(12), 200),
        (CityScale::Medium.config(12), 400),
        (CityScale::Large.config(12), 800),
        (
            CityGenerationConfig {
                seed: 12,
                districts: 24,
                businesses_per_district: 40,
                rivals: 8,
            },
            1_100,
        ),
    ];
    for (config, minimum) in targets {
        let city = generate_city(&registry, config).unwrap();
        let world = city.state.world();
        assert!(
            world.population_size() >= minimum,
            "{} districts should have at least {minimum} people, got {}",
            config.districts,
            world.population_size()
        );
        let mut names = std::collections::BTreeSet::new();
        let mut freelancers = 0;
        let mut employees = 0;
        let mut police = 0;
        let mut politicians = 0;
        let mut lawyers = 0;
        let mut journalists = 0;
        for person in world.characters() {
            assert!(
                names.insert(person.name().to_owned()),
                "distinct inhabitants cannot have duplicated full names"
            );
            match world.civic_profession(person.id()).unwrap() {
                CivicProfession::FreelanceCriminal => freelancers += 1,
                CivicProfession::BusinessEmployee => employees += 1,
                CivicProfession::PoliceOfficer => police += 1,
                CivicProfession::Politician => politicians += 1,
                CivicProfession::Lawyer | CivicProfession::IndependentLawyer => lawyers += 1,
                CivicProfession::Journalist => journalists += 1,
                CivicProfession::Criminal
                | CivicProfession::Resident
                | CivicProfession::BusinessOwner
                | CivicProfession::Prosecutor
                | CivicProfession::LaborRepresentative
                | CivicProfession::MerchantRepresentative => {}
            }
            if person.life().home().is_some() {
                assert!(
                    world
                        .get_neighborhood(person.life().home().unwrap())
                        .is_some()
                );
            }
        }
        let wards = usize::from(config.districts);
        assert!(freelancers >= wards * 9);
        assert!(employees >= wards * 12);
        assert!(police >= wards * 4);
        assert!(politicians >= wards * 3);
        assert!(lawyers >= wards * 3);
        assert!(journalists >= wards);
        assert!(
            world
                .businesses()
                .any(|record| matches!(record.owner(), crate::world::BusinessOwner::Independent)),
            "public listings must remain accessible to new criminal entrepreneurs"
        );
        validate_invariants(&city.state);
        validate_state_against_registry(&registry, &city.state).unwrap();
        let loaded = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        assert_eq!(loaded.world().population_size(), world.population_size());
        let again = generate_city(&registry, config).unwrap();
        assert_eq!(
            bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap(),
            bincode::serialize(&build_save(&registry, &again.state).unwrap()).unwrap(),
            "population and all social ties must follow only the supplied city seed"
        );
    }
}

#[test]
fn paid_freelance_contract_is_personal_temporary_and_recruitable_after_expiry() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562365)).unwrap();
    let freelancer = city
        .state
        .world()
        .characters()
        .find(|person| {
            person.life().vocation() == IndependentVocation::FreelanceCriminal
                && city
                    .state
                    .social()
                    .get_relationship(person.id(), city.boss)
                    .is_some()
        })
        .unwrap()
        .id();
    let manager = city.boss;
    let now = city.state.now();
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(matches!(
        validate_hire_freelancer(&city.state, city.player, freelancer, manager, 0),
        Err(FreelanceError::InvalidDuration)
    ));
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    let org_money_before: i128 = city
        .state
        .finance()
        .accounts_for(crate::finance::FinancialOwner::Organization(city.player))
        .map(|account| i128::from(account.balance().cents()))
        .sum();
    let signed =
        validate_hire_freelancer(&city.state, city.player, freelancer, manager, 2).unwrap();
    let fee = signed.fee_cents();
    signed.commit(&mut city.state).unwrap();
    let contract = city
        .state
        .world()
        .active_freelance_contract(freelancer)
        .unwrap();
    assert_eq!(contract.signed_at(), now);
    assert_eq!(
        contract.expires_at().as_minutes(),
        now.as_minutes() + 2 * DAY_MINUTES
    );
    assert_eq!(
        city.state
            .world()
            .get_character(freelancer)
            .unwrap()
            .organization(),
        Some(city.player)
    );
    assert_eq!(
        city.state
            .world()
            .get_character(freelancer)
            .unwrap()
            .supervisor(),
        Some(manager)
    );
    let org_money_after: i128 = city
        .state
        .finance()
        .accounts_for(crate::finance::FinancialOwner::Organization(city.player))
        .map(|account| i128::from(account.balance().cents()))
        .sum();
    let personal_cash: i128 = city
        .state
        .finance()
        .accounts_for(crate::finance::FinancialOwner::Character(freelancer))
        .map(|account| i128::from(account.balance().cents()))
        .sum();
    assert_eq!(org_money_before - org_money_after, i128::from(fee));
    assert_eq!(personal_cash, i128::from(fee));
    assert!(matches!(
        validate_hire_freelancer(&city.state, city.player, freelancer, manager, 2),
        Err(FreelanceError::Unavailable(_)) | Err(FreelanceError::AlreadyContracted)
    ));
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(
        restored
            .world()
            .active_freelance_contract(freelancer)
            .is_some()
    );

    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    assert!(apply_due_freelance_expirations(&mut city.state).is_empty());
    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    assert_eq!(
        apply_due_freelance_expirations(&mut city.state),
        vec![freelancer]
    );
    assert_eq!(
        city.state
            .world()
            .get_character(freelancer)
            .unwrap()
            .organization(),
        None
    );
    assert!(
        city.state
            .world()
            .active_freelance_contract(freelancer)
            .is_none()
    );
    let history: Vec<_> = city.state.world().freelance_contracts().collect();
    assert!(
        history
            .iter()
            .any(|record| record.freelancer() == freelancer && record.ended_at().is_some())
    );
    // A finished contract creates a social recruitment route, not permanent
    // membership. The freelancer can still accept/refuse a standard pitch.
    validate_recruitment_attempt(
        &registry,
        &city.state,
        RecruitmentDraft {
            target_organization: city.player,
            recruiter: manager,
            candidate: freelancer,
            approach: RecruitmentApproach::FinancialOpportunity,
        },
    )
    .expect("formerly contracted freelancer must be considered by the real recruitment owner");
    validate_invariants(&city.state);
    restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
}

#[test]
fn freelance_expiration_uses_its_signed_minute_without_charging_an_extra_day() {
    use crate::core::simulation::run_tick;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let freelancer = city
        .state
        .world()
        .characters()
        .find(|person| {
            person.life().vocation() == IndependentVocation::FreelanceCriminal
                && city
                    .state
                    .social()
                    .get_relationship(person.id(), city.boss)
                    .is_some()
        })
        .unwrap()
        .id();

    run_tick(&registry, &mut city.state).unwrap(); // Signed one minute after midnight.
    validate_hire_freelancer(&city.state, city.player, freelancer, city.boss, 1)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let deadline = city
        .state
        .world()
        .active_freelance_contract(freelancer)
        .unwrap()
        .expires_at();
    assert_eq!(deadline.as_minutes(), DAY_MINUTES + 1);

    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32 - 2));
    let midnight = run_tick(&registry, &mut city.state).unwrap();
    assert_eq!(city.state.now().as_minutes(), DAY_MINUTES);
    assert!(
        midnight
            .payrolls
            .iter()
            .any(|payroll| payroll.organization() == city.player
                && payroll.owed().cents() >= registry.upkeep().per_member_daily().cents())
    );
    assert!(
        city.state
            .world()
            .active_freelance_contract(freelancer)
            .is_some()
    );

    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let ending = run_tick(&registry, &mut city.state).unwrap();
    let restored_ending = run_tick(&registry, &mut restored).unwrap();
    assert_eq!(ending.freelance_contract_expirations, vec![freelancer]);
    assert_eq!(
        restored_ending.freelance_contract_expirations,
        vec![freelancer]
    );
    assert_eq!(city.state.now(), deadline);
    assert!(
        city.state
            .world()
            .active_freelance_contract(freelancer)
            .is_none(),
        "a signed one-day contract must not remain on payroll for a second day"
    );
    let ended = city
        .state
        .world()
        .freelance_contracts()
        .find(|contract| contract.freelancer() == freelancer)
        .unwrap();
    assert_eq!(ended.ended_at(), Some(deadline));
    assert_eq!(
        city.state
            .world()
            .get_character(freelancer)
            .unwrap()
            .organization(),
        None
    );
    validate_invariants(&city.state);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
