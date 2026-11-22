use super::*;
use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
use crate::core::campaign_victory::{VictoryError, VictoryPath, validate_claim_victory};
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::politics::political_system::{
    PoliticalError, apply_due_political_campaigns, initiative_available, validate_civic_initiative,
    validate_nominate_mayor,
};
use crate::world::{AutonomyLevel, OrganizationKind};

fn fund(state: &mut crate::AppState, owner: OrganizationId, cents: i64) {
    let source = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let destination = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Political fixture capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: destination,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
}

fn day(registry: &crate::Registry, state: &mut crate::AppState) {
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(registry, state).unwrap();
    apply_due_political_campaigns(registry, state);
}

#[test]
fn local_initiatives_require_real_venues_pay_multiple_accounts_and_observe_cooldowns() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let district = city.home;
    assert!(initiative_available(
        &city.state,
        city.player,
        district,
        CivicInitiative::PublicMeetings
    ));
    assert!(!initiative_available(
        &city.state,
        city.player,
        district,
        CivicInitiative::LocalPress
    ));
    fund(&mut city.state, city.player, 100_000);
    fund(&mut city.state, city.player, 100_000);
    fund(&mut city.state, city.player, 5_000_000);
    let manager = city
        .state
        .world()
        .characters_in_organization(city.player)
        .find(|person| {
            person.supervisor().is_some()
                && city
                    .state
                    .delegation()
                    .active_for_manager(person.id())
                    .is_some()
        })
        .unwrap()
        .id();
    let mandate = city.state.delegation().active_for_manager(manager).unwrap();
    let mut scopes = mandate.scopes().clone();
    scopes.insert(ResponsibilityScope::Neighborhood(district));
    scopes.insert(ResponsibilityScope::Function(
        ResponsibilityFunction::Political,
    ));
    validate_revise_mandate(
        &city.state,
        mandate.id(),
        MandateRevisionDraft {
            scopes,
            standing_orders: mandate.standing_orders().clone(),
            budget: mandate.budget(),
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    if city
        .state
        .world()
        .get_character(manager)
        .unwrap()
        .autonomy()
        != AutonomyLevel::Delegated
    {
        crate::world::world_system::validate_set_character_autonomy(
            &city.state,
            city.player,
            manager,
            AutonomyLevel::Delegated,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    let unchanged = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_civic_initiative(&city.state, district, manager, CivicInitiative::LocalPress)
            .err(),
        Some(PoliticalError::MissingCivicInfrastructure)
    );
    assert_eq!(
        unchanged,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );

    // Even at the machine-influence cap, the campaign must be able to
    // organize actual residents and institutions instead of hitting a dead end.
    political_system::validate_invest_influence(&city.state, district, 5_000_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_civic_initiative(
        &city.state,
        district,
        manager,
        CivicInitiative::PublicMeetings,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let office = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .district(district)
        .unwrap();
    assert_eq!(
        (
            office.influence(),
            office.grassroots(),
            office.institutions()
        ),
        (1_000, 70, 0)
    );
    assert_eq!(
        validate_civic_initiative(
            &city.state,
            district,
            manager,
            CivicInitiative::PublicMeetings
        )
        .err(),
        Some(PoliticalError::InitiativeCooldown)
    );
    let municipal = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let paid: i64 = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(municipal))
        .filter(|account| account.kind() == AccountKind::Settlement)
        .map(|account| account.balance().cents())
        .sum();
    assert_eq!(
        paid, 5_150_000,
        "a political expense must leave the criminal organization's books"
    );
    for _ in 0..7 {
        day(&registry, &mut city.state);
    }
    let unchanged = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_civic_initiative(
            &city.state,
            district,
            manager,
            CivicInitiative::PublicMeetings
        )
        .err(),
        Some(PoliticalError::InsufficientFunds)
    );
    assert_eq!(
        unchanged,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    fund(&mut city.state, city.player, 100_000);
    validate_civic_initiative(
        &city.state,
        district,
        manager,
        CivicInitiative::PublicMeetings,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn nomination_requires_a_real_sponsor_and_elections_record_loss_with_deterministic_continuation() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    fund(&mut city.state, city.player, 5_000_000);
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::IneligibleCandidate)
    );
    validate_establish_contact(
        &city.state,
        InstitutionalContactDraft {
            sponsor: city.player,
            handler: city.boss,
            contact: official,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    for _ in 0..30 {
        day(&registry, &mut city.state);
    }
    validate_nominate_mayor(&city.state, city.boss)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::AlreadyNominated)
    );
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .unwrap()
            .election_day,
        180
    );
    for _ in 30..179 {
        day(&registry, &mut city.state);
    }
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    day(&registry, &mut city.state);
    day(&registry, &mut restored);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    let election = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .latest_election()
        .unwrap();
    assert_eq!(election.election_day, 180);
    assert_eq!(election.candidate, city.boss);
    assert!(!election.elected);
    assert_eq!(election.wards_won, 0);
    assert_eq!(
        election.ballots.len(),
        city.state.world().neighborhoods().count()
    );
    assert!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .is_none()
    );
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    validate_nominate_mayor(&city.state, city.boss)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .unwrap()
            .election_day,
        360
    );
    validate_invariants(&city.state);
}
