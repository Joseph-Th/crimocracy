//! Territory-influence derivation, scope, and deterministic ordering tests.

use super::*;
use crate::build_registry;
use crate::core::id::{FinancialAccountId, MandateId};
use crate::core::invariants::validate_invariants;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
use crate::enterprises::enterprise_execution::{
    validate_establish_enterprise, validate_suspend_enterprise,
};
use crate::enterprises::{EnterpriseDraft, EnterpriseKind};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::registry::Registry;
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::BTreeSet;

struct InfluenceFixture {
    state: AppState,
    neighborhood: NeighborhoodId,
    dominant: OrganizationId,
    challenger: OrganizationId,
    /// The dominant organization's district-scoped mandate.
    mandate: MandateId,
    manager: crate::core::id::CharacterId,
}

fn assign_district_authority(
    state: &mut AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    manager_name: &str,
) -> (crate::core::id::CharacterId, MandateId) {
    let manager = insert_character(
        state,
        CharacterDraft {
            name: manager_name.to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("district manager should validate");
    let mandate = validate_assign_mandate(
        state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("district mandate should validate")
    .commit(state)
    .expect("district mandate should commit");
    (manager, mandate)
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_influence_fixture() -> InfluenceFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x7E27_1A11);
    let dominant = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Dominant Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("dominant organization should validate");
    let challenger = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Challenger Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("challenger organization should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Influence Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(55),
                    commercial_activity: rating(60),
                    illicit_demand: rating(45),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "District Lieutenant".to_owned(),
            organization: Some(dominant),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: dominant,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate should validate")
    .commit(&mut state)
    .expect("mandate should commit");
    InfluenceFixture {
        state,
        neighborhood,
        dominant,
        challenger,
        mandate,
        manager,
    }
}

fn org_cash(state: &mut AppState, organization: OrganizationId) -> FinancialAccountId {
    insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("cash account should validate")
}

fn fresh_settlement(state: &mut AppState, organization: OrganizationId) -> FinancialAccountId {
    insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate")
}

fn insert_venue(
    registry: &Registry,
    state: &mut AppState,
    name: &str,
    neighborhood: NeighborhoodId,
    owner: BusinessOwner,
    functions: BTreeSet<BusinessFunction>,
) -> crate::core::id::BusinessId {
    insert_business(
        registry,
        state,
        BusinessDraft {
            name: name.to_owned(),
            kind: BusinessKind::Hospitality,
            functions,
            neighborhood,
            owner,
        },
    )
    .expect("venue should validate")
}

fn establish_gambling_at_venue(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    mandate: MandateId,
    manager: crate::core::id::CharacterId,
    venue: crate::core::id::BusinessId,
) -> crate::core::id::EnterpriseId {
    let scope = ResponsibilityScope::Neighborhood(
        state
            .world()
            .get_business(venue)
            .expect("hosted venue should exist")
            .neighborhood(),
    );
    let cash_account = org_cash(state, organization);
    let settlement_account = fresh_settlement(state, organization);
    validate_establish_enterprise(
        registry,
        state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: MandateAuthority {
                mandate,
                manager,
                scope,
            },
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .expect("gambling establishment should validate")
    .commit(state)
    .expect("gambling establishment should commit")
}

fn hospitality_functions() -> BTreeSet<BusinessFunction> {
    BTreeSet::from([
        BusinessFunction::CashIntensive,
        BusinessFunction::MeetingSpace,
        BusinessFunction::CustomerAccess,
    ])
}

#[test]
fn influence_counts_active_district_rackets_per_organization() {
    let registry = build_registry();
    let mut fixture = make_influence_fixture();

    let dominant_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Dominant Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.dominant),
        hospitality_functions(),
    );
    // A challenger-owned venue with no racket projects no economic presence even though
    // property exists in the district.
    let _challenger_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Challenger Corner",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.dominant,
        fixture.mandate,
        fixture.manager,
        dominant_venue,
    );

    let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
        .expect("influence should resolve");
    assert_eq!(
        summary.standings.len(),
        1,
        "only the racket operator holds economic presence"
    );
    assert_eq!(summary.standings[0].organization, fixture.dominant);
    assert_eq!(summary.standings[0].active_enterprises, 1);

    assert_eq!(
        summary.economic_leader(),
        Some(fixture.dominant),
        "the sole racket operator leads an uncontested district"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn contested_districts_have_no_economic_leader() {
    let registry = build_registry();
    let mut fixture = make_influence_fixture();

    // A second district-scoped mandate lets the challenger operate here too.
    let (challenger_manager, challenger_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Challenger Lieutenant",
    );

    for (organization, mandate, manager, name) in [
        (
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            "Dominant Hall",
        ),
        (
            fixture.challenger,
            challenger_mandate,
            challenger_manager,
            "Challenger Corner",
        ),
    ] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(organization),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            organization,
            mandate,
            manager,
            venue,
        );
    }

    let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
        .expect("influence should resolve");
    assert_eq!(
        summary.economic_leader(),
        None,
        "one racket each: contested"
    );
    for standing in &summary.standings {
        assert_eq!(
            standing.active_enterprises, 1,
            "both sides hold exactly one district racket"
        );
    }
    validate_invariants(&fixture.state);
}

#[test]
fn strict_higher_enterprise_count_replaces_earlier_leader() {
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let trailing = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Trailing Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("trailing organization should validate");
    let (challenger_manager, challenger_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Challenger Lieutenant",
    );
    let (trailing_manager, trailing_mandate) = assign_district_authority(
        &mut fixture.state,
        trailing,
        fixture.neighborhood,
        "Trailing Lieutenant",
    );

    for (organization, mandate, manager, venue_names) in [
        (
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            &["Dominant Hall"][..],
        ),
        (
            fixture.challenger,
            challenger_mandate,
            challenger_manager,
            &["Challenger Hall", "Challenger Annex"][..],
        ),
        (
            trailing,
            trailing_mandate,
            trailing_manager,
            &["Trailing Hall"][..],
        ),
    ] {
        for name in venue_names {
            let venue = insert_venue(
                &registry,
                &mut fixture.state,
                name,
                fixture.neighborhood,
                BusinessOwner::Organization(organization),
                hospitality_functions(),
            );
            establish_gambling_at_venue(
                &registry,
                &mut fixture.state,
                organization,
                mandate,
                manager,
                venue,
            );
        }
    }

    let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
        .expect("influence should resolve");
    assert_eq!(
        summary
            .standings
            .iter()
            .map(|standing| (standing.organization, standing.active_enterprises))
            .collect::<Vec<_>>(),
        vec![
            (fixture.dominant, 1),
            (fixture.challenger, 2),
            (trailing, 1),
        ],
        "fixture must exercise a later strict leader followed by a lower trailing standing"
    );
    assert_eq!(
        summary.economic_leader(),
        Some(fixture.challenger),
        "the unique highest enterprise count must replace an earlier lower-count leader"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_raids_prefer_a_safer_rival_over_an_earlier_created_target() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::world::territory_conflict::apply_due_autonomous_territory_clashes;

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let weaker = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Weaker Rival".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("third organization should validate");
    let (strong_manager, strong_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Stronger Rival Manager",
    );
    let (weak_manager, weak_mandate) = assign_district_authority(
        &mut fixture.state,
        weaker,
        fixture.neighborhood,
        "Weaker Rival Manager",
    );
    let mut stronger_targets = Vec::new();
    let mut weaker_target = None;

    // The first eligible rival has two older rackets. Both rivals are beatable,
    // but the later, single-front rival offers a far larger known force advantage.
    for (organization, mandate, manager, count, prefix) in [
        (
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            3,
            "Attacking",
        ),
        (
            fixture.challenger,
            strong_mandate,
            strong_manager,
            2,
            "Stronger",
        ),
        (weaker, weak_mandate, weak_manager, 1, "Weaker"),
    ] {
        for index in 0..count {
            let venue = insert_venue(
                &registry,
                &mut fixture.state,
                &format!("{prefix} Hall {index}"),
                fixture.neighborhood,
                BusinessOwner::Organization(organization),
                hospitality_functions(),
            );
            let racket = establish_gambling_at_venue(
                &registry,
                &mut fixture.state,
                organization,
                mandate,
                manager,
                venue,
            );
            if organization == fixture.challenger {
                stronger_targets.push(racket);
            } else if organization == weaker {
                weaker_target = Some(racket);
            }
        }
    }

    let weaker_target = weaker_target.expect("weaker organization has a racket");
    assert!(stronger_targets.iter().all(|&id| id < weaker_target));
    fixture.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    let clashes = apply_due_autonomous_territory_clashes(&mut fixture.state);
    assert_eq!(clashes.len(), 1);
    assert!(clashes[0].attacker_won);
    assert_eq!(clashes[0].suspended_racket, weaker_target);
    assert!(stronger_targets.iter().all(|&id| {
        fixture
            .state
            .enterprises()
            .get_enterprise(id)
            .is_some_and(|racket| racket.status() == EnterpriseStatus::Active)
    }));
    validate_invariants(&fixture.state);

    let saved = build_save(&registry, &fixture.state).expect("clash state should save");
    let restored = restore_save(&registry, saved).expect("clash state should restore");
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(weaker_target)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended,
    );
    validate_invariants(&restored);
}

#[test]
fn autonomous_raids_reassess_own_losses_before_later_same_day_choices() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::enterprises::enterprise_execution::validate_staff_enterprise;
    use crate::world::territory_conflict::{
        active_district_force, apply_due_autonomous_territory_clashes,
    };
    use crate::world::world_system::designate_player_organization;

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let player = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Neutral Player".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut fixture.state, player).unwrap();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Second Rival Captain",
    );
    let (player_manager, player_mandate) = assign_district_authority(
        &mut fixture.state,
        player,
        fixture.neighborhood,
        "Player Captain",
    );
    let mut rival_rackets = Vec::new();
    // All three gangs have the same visible baseline (10). Create the second
    // rival's fronts first to make it the first gang's preferred equal-risk target.
    for (organization, manager, mandate, staffing) in [
        (fixture.challenger, rival_manager, rival_mandate, [9, 5]),
        (fixture.dominant, fixture.manager, fixture.mandate, [11, 11]),
        (player, player_manager, player_mandate, [5, 5]),
    ] {
        for (index, associates) in staffing.into_iter().enumerate() {
            let venue = insert_venue(
                &registry,
                &mut fixture.state,
                &format!("{organization} Club {index}"),
                fixture.neighborhood,
                BusinessOwner::Organization(organization),
                hospitality_functions(),
            );
            let racket = establish_gambling_at_venue(
                &registry,
                &mut fixture.state,
                organization,
                mandate,
                manager,
                venue,
            );
            if associates != EnterpriseKind::Gambling.baseline_associates() {
                validate_staff_enterprise(&fixture.state, racket, associates)
                    .unwrap()
                    .commit(&mut fixture.state)
                    .unwrap();
            }
            if organization == fixture.challenger {
                rival_rackets.push(racket);
            }
        }
    }
    assert_eq!(
        active_district_force(&fixture.state, fixture.challenger, fixture.neighborhood),
        14
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    let clashes = apply_due_autonomous_territory_clashes(&mut fixture.state);
    assert_eq!(
        clashes.len(),
        1,
        "a gang reduced to five associates must not act on its obsolete fourteen-person estimate"
    );
    assert_eq!(
        apply_due_autonomous_territory_clashes(&mut restored),
        clashes
    );
    assert_eq!(
        bincode::serialize(&restored).unwrap(),
        bincode::serialize(&fixture.state).unwrap()
    );
    assert!(clashes[0].attacker_won);
    assert_eq!(clashes[0].suspended_racket, rival_rackets[0]);
    assert_eq!(
        active_district_force(&fixture.state, fixture.challenger, fixture.neighborhood),
        5
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(rival_rackets[1])
            .unwrap()
            .status(),
        EnterpriseStatus::Active
    );
    validate_invariants(&fixture.state);
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn suspended_rackets_leave_the_footprint() {
    let registry = build_registry();
    let mut fixture = make_influence_fixture();

    let venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Dominant Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.dominant),
        hospitality_functions(),
    );
    let enterprise = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.dominant,
        fixture.mandate,
        fixture.manager,
        venue,
    );
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("suspension should validate")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
        .expect("influence should resolve");
    assert!(
        summary.standings.is_empty(),
        "a suspended racket projects no influence and there is no other footprint"
    );
    assert_eq!(summary.economic_leader(), None);
    validate_invariants(&fixture.state);
}

#[test]
fn unknown_districts_are_rejected_without_state_change() {
    let registry = build_registry();
    let fixture = make_influence_fixture();
    let _ = registry;
    // An unallocated id of the right shape never resolves.
    let missing = crate::core::id::NeighborhoodId::from_raw(9_999);
    let error = match resolve_neighborhood_influence(&fixture.state, missing) {
        Err(error) => error,
        Ok(_) => panic!("unknown districts must be rejected"),
    };
    assert_eq!(error, TerritoryInfluenceError::MissingNeighborhood(missing));
}

#[test]
fn negotiated_standstill_is_symmetric_durable_and_expires_without_changing_existing_rackets() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::enterprises::enterprise_execution::EnterpriseError;
    use crate::history::HistoryEventKind;

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Rival Envoy",
    );
    assert_eq!(
        validate_negotiate_standstill(
            &fixture.state,
            fixture.dominant,
            fixture.challenger,
            fixture.neighborhood,
        )
        .err(),
        Some(TerritoryNegotiationError::NoContestedPresence),
    );

    let first_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "First Corner",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.dominant),
        hospitality_functions(),
    );
    let second_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Rival Corner",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    let first_racket = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.dominant,
        fixture.mandate,
        fixture.manager,
        first_venue,
    );
    let rival_racket = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.challenger,
        rival_mandate,
        rival_manager,
        second_venue,
    );
    let stale = validate_negotiate_standstill(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
    )
    .expect("evenly contested rivals can agree");
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    assert!(matches!(
        stale.commit(&mut fixture.state),
        Err(TerritoryNegotiationError::StaleTime { .. })
    ));
    assert!(active_standstill(&fixture.state, fixture.dominant, fixture.neighborhood).is_none());

    let agreed = validate_negotiate_standstill(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
    )
    .expect("same-minute negotiation should validate");
    let history = agreed
        .commit(&mut fixture.state)
        .expect("signed accord persists");
    assert!(matches!(
        fixture.state.history().get_event(history).unwrap().kind(),
        HistoryEventKind::TerritoryStandstill { .. }
    ));
    for (organization, opponent) in [
        (fixture.dominant, fixture.challenger),
        (fixture.challenger, fixture.dominant),
    ] {
        let treaty = active_standstill(&fixture.state, organization, fixture.neighborhood)
            .expect("accord applies symmetrically");
        assert_eq!(treaty.counterparty, opponent);
        assert_eq!(treaty.history, history);
    }
    assert_eq!(
        validate_negotiate_standstill(
            &fixture.state,
            fixture.challenger,
            fixture.dominant,
            fixture.neighborhood,
        )
        .err(),
        Some(TerritoryNegotiationError::ExistingStandstill)
    );
    for (organization, manager, mandate, venue) in [
        (
            fixture.dominant,
            fixture.manager,
            fixture.mandate,
            first_venue,
        ),
        (
            fixture.challenger,
            rival_manager,
            rival_mandate,
            second_venue,
        ),
    ] {
        let cash = org_cash(&mut fixture.state, organization);
        let settlement = fresh_settlement(&mut fixture.state, organization);
        let rejected = validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::LoanSharking,
                organization,
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope: ResponsibilityScope::Neighborhood(fixture.neighborhood),
                },
                location: EnterpriseLocation::Business(venue),
                supporting_businesses: BTreeSet::new(),
                cash_account: cash,
                settlement_account: settlement,
            },
        )
        .err()
        .expect("a pact forbids either gang opening another racket");
        assert_eq!(
            rejected,
            EnterpriseError::TerritoryStandstill(fixture.neighborhood)
        );
    }
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(first_racket)
            .unwrap()
            .status(),
        crate::enterprises::EnterpriseStatus::Active,
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(rival_racket)
            .unwrap()
            .status(),
        crate::enterprises::EnterpriseStatus::Active,
    );
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).expect("agreement is a valid saved state");
    let bytes = bincode::serialize(&saved).expect("serialize agreed campaign");
    let restored = restore_save(&registry, bincode::deserialize(&bytes).unwrap())
        .expect("restore agreed campaign");
    assert!(active_standstill(&restored, fixture.challenger, fixture.neighborhood).is_some());

    fixture.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(STANDSTILL_DAYS * DAY_MINUTES).expect("bounded agreement duration"),
    ));
    assert!(active_standstill(&fixture.state, fixture.dominant, fixture.neighborhood).is_none());
    assert!(active_standstill(&fixture.state, fixture.challenger, fixture.neighborhood).is_none());
    validate_invariants(&fixture.state);
}

#[test]
fn bilateral_standstill_does_not_disarm_either_gang_against_a_third_party() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::world::territory_conflict::{TerritoryConflictError, validate_territory_clash};
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let third = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Unaffiliated Third Gang".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("third gang must exist");
    let (second_manager, second_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Ceasefire Counterparty",
    );
    let (third_manager, third_mandate) = assign_district_authority(
        &mut fixture.state,
        third,
        fixture.neighborhood,
        "Unaffiliated Captain",
    );
    let mut rackets = Vec::new();
    for (organization, manager, mandate, name) in [
        (
            fixture.dominant,
            fixture.manager,
            fixture.mandate,
            "First Club",
        ),
        (
            fixture.challenger,
            second_manager,
            second_mandate,
            "Second Club",
        ),
        (third, third_manager, third_mandate, "Third Club"),
    ] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(organization),
            hospitality_functions(),
        );
        rackets.push(establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            organization,
            mandate,
            manager,
            venue,
        ));
    }
    validate_negotiate_standstill(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
    )
    .expect("first two gangs can negotiate")
    .commit(&mut fixture.state)
    .expect("bilateral ceasefire must commit");
    assert!(matches!(
        validate_territory_clash(&fixture.state, fixture.dominant, rackets[1]),
        Err(TerritoryConflictError::Standstill(district)) if district == fixture.neighborhood
    ));
    validate_territory_clash(&fixture.state, fixture.dominant, rackets[2])
        .expect("a gang can still confront an unrelated third party");
    let attack = validate_territory_clash(&fixture.state, third, rackets[0])
        .expect("the unaffiliated third gang must remain able to attack a signatory");
    let outcome = attack
        .commit(&mut fixture.state)
        .expect("third-party clash must commit");
    assert!(!outcome.attacker_won);
    assert_eq!(outcome.suspended_racket, rackets[2]);
    assert!(active_standstill(&fixture.state, fixture.dominant, fixture.neighborhood).is_some());
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).expect("third-party clash must save");
    let restored = restore_save(&registry, saved).expect("third-party clash must restore");
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}

#[test]
fn a_failed_turf_challenge_closes_the_attacker_and_recovers_after_seven_days() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::enterprises::enterprise_execution::{EnterpriseError, validate_resume_enterprise};
    use crate::world::territory_conflict::{
        TerritoryConflictError, suppression_ends_at, validate_territory_clash,
    };
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Rival Guard",
    );
    let own_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Own Front",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.dominant),
        hospitality_functions(),
    );
    let rival_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Rival Front",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    let own = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.dominant,
        fixture.mandate,
        fixture.manager,
        own_venue,
    );
    let rival = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.challenger,
        rival_mandate,
        rival_manager,
        rival_venue,
    );
    let outcome = validate_territory_clash(&fixture.state, fixture.dominant, rival)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(!outcome.attacker_won, "equal manpower favors defense");
    assert_eq!(outcome.suspended_racket, own);
    assert!(suppression_ends_at(&fixture.state, own).is_some());
    assert!(matches!(
        validate_resume_enterprise(&registry, &fixture.state, own),
        Err(EnterpriseError::TerritorySuppressed { .. })
    ));
    assert!(matches!(
        validate_territory_clash(&fixture.state, fixture.dominant, rival),
        Err(TerritoryConflictError::Cooldown(_))
    ));
    // The loser still has a protected claim while its racket is forcibly closed.
    // Peace should be negotiable immediately after the fighting instead of
    // requiring the loser to resume first.
    validate_negotiate_standstill(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
    )
    .expect("recently displaced gang can negotiate")
    .commit(&mut fixture.state)
    .expect("post-clash standstill signs");
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let bytes = bincode::serialize(&saved).unwrap();
    let restored = restore_save(&registry, bincode::deserialize(&bytes).unwrap()).unwrap();
    assert!(suppression_ends_at(&restored, own).is_some());
    fixture.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(7 * DAY_MINUTES).unwrap(),
    ));
    assert!(suppression_ends_at(&fixture.state, own).is_none());
    validate_resume_enterprise(&registry, &fixture.state, own)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(
        active_standstill(&fixture.state, fixture.dominant, fixture.neighborhood).is_some(),
        "a standstill must not prevent reopening an existing racket"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn stronger_npc_territorial_force_displaces_player_racket_and_reports_crisis() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::world::territory_conflict::{
        apply_due_autonomous_territory_clashes, suppression_ends_at,
    };
    use crate::world::world_system::designate_player_organization;
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    designate_player_organization(&mut fixture.state, fixture.challenger).unwrap();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Defending Captain",
    );
    for name in ["Offensive Hall", "Offensive Annex"] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.dominant),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            venue,
        );
    }
    let target_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Defender Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    let target = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.challenger,
        rival_mandate,
        rival_manager,
        target_venue,
    );
    fixture.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    let clashes = apply_due_autonomous_territory_clashes(&mut fixture.state);
    assert_eq!(clashes.len(), 1);
    assert!(clashes[0].attacker_won);
    assert_eq!(clashes[0].suspended_racket, target);
    assert_eq!(
        resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
            .unwrap()
            .economic_leader(),
        Some(fixture.dominant)
    );
    assert!(suppression_ends_at(&fixture.state, target).is_some());
    let reports: Vec<_> = fixture
        .state
        .reports()
        .reports_for(fixture.challenger)
        .filter(|report| report.title().contains("Turf conflict"))
        .collect();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].entries()[0].attention,
        crate::core::attention::AttentionClass::Crisis
    );
    assert!(apply_due_autonomous_territory_clashes(&mut fixture.state).is_empty());
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let bytes = bincode::serialize(&saved).unwrap();
    let restored = restore_save(&registry, bincode::deserialize(&bytes).unwrap()).unwrap();
    assert!(suppression_ends_at(&restored, target).is_some());
}

#[test]
fn extra_defenders_can_repulse_an_optimistic_rival_ai_and_deter_repeat_raids() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::enterprises::enterprise_execution::{
        validate_resume_enterprise, validate_staff_enterprise,
    };
    use crate::world::territory_conflict::apply_due_autonomous_territory_clashes;
    use crate::world::world_system::designate_player_organization;
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    designate_player_organization(&mut fixture.state, fixture.challenger).unwrap();
    let (defender_manager, defender_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Defending Captain",
    );
    let mut attacking_fronts = Vec::new();
    for name in ["Attacker First", "Attacker Second"] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.dominant),
            hospitality_functions(),
        );
        attacking_fronts.push(establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            venue,
        ));
    }
    let defended_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Defensive Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    let defended = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.challenger,
        defender_mandate,
        defender_manager,
        defended_venue,
    );
    validate_staff_enterprise(&fixture.state, defended, 13)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let day = SimDuration::from_minutes(u32::try_from(DAY_MINUTES).unwrap());
    fixture.state.advance_clock(day);
    let clash = apply_due_autonomous_territory_clashes(&mut fixture.state);
    assert_eq!(
        clash.len(),
        1,
        "rival overestimates itself using baseline defender staffing"
    );
    assert!(!clash[0].attacker_won);
    assert!(attacking_fronts.contains(&clash[0].suspended_racket));
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(defended)
            .unwrap()
            .status(),
        crate::enterprises::EnterpriseStatus::Active
    );
    fixture.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(7 * DAY_MINUTES).unwrap(),
    ));
    validate_resume_enterprise(&registry, &fixture.state, clash[0].suspended_racket)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    // The old baseline estimate still suggests an easy target, but this
    // leader remembers the repelled raid and will not repeat it immediately.
    assert!(apply_due_autonomous_territory_clashes(&mut fixture.state).is_empty());
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let mut restored = restore_save(
        &registry,
        bincode::deserialize(&bincode::serialize(&saved).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(
        apply_due_autonomous_territory_clashes(&mut restored).is_empty(),
        "restoration must rebuild failed-raid deterrence without rerunning history"
    );
}

#[test]
fn an_outnumbered_gang_can_buy_peace_with_real_street_cash() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::finance::finance_system::validate_record_transaction;
    use crate::finance::{LedgerPosting, LedgerTransactionDraft, Money};
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Rival Lieutenant",
    );
    let own_venue = insert_venue(
        &registry,
        &mut fixture.state,
        "Own Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.dominant),
        hospitality_functions(),
    );
    establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.dominant,
        fixture.mandate,
        fixture.manager,
        own_venue,
    );
    for name in ["Rival Hall", "Rival Annex", "Rival Outpost"] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.challenger),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.challenger,
            rival_mandate,
            rival_manager,
            venue,
        );
    }
    assert_eq!(
        validate_negotiate_standstill(
            &fixture.state,
            fixture.dominant,
            fixture.challenger,
            fixture.neighborhood,
        )
        .err(),
        Some(TerritoryNegotiationError::TributeRequired {
            required_cents: 25_000,
            offered_cents: 0,
        })
    );
    assert_eq!(
        validate_negotiate_standstill_with_tribute(
            &fixture.state,
            fixture.dominant,
            fixture.challenger,
            fixture.neighborhood,
            Money::from_cents(25_000),
        )
        .err(),
        Some(TerritoryNegotiationError::InsufficientTributeFunds)
    );

    let source = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(fixture.dominant))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .unwrap()
        .id();
    let destination = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(fixture.challenger))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .unwrap()
        .id();
    let clearing = fresh_settlement(&mut fixture.state, fixture.dominant);
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fixture funds for tribute".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(100_000),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-100_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let offer = validate_negotiate_standstill_with_tribute(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
        Money::from_cents(25_000),
    )
    .expect("real street-cash tribute satisfies stronger rival");
    let history = offer
        .commit(&mut fixture.state)
        .expect("payment and treaty commit together");
    assert!(fixture.state.history().get_event(history).is_some());
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(source)
            .unwrap()
            .balance()
            .cents(),
        75_000
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(destination)
            .unwrap()
            .balance()
            .cents(),
        25_000
    );
    assert!(active_standstill(&fixture.state, fixture.challenger, fixture.neighborhood).is_some());
    assert_eq!(
        fixture
            .state
            .finance()
            .get_transaction(crate::core::id::LedgerTransactionId::from_raw(2))
            .unwrap()
            .postings()
            .len(),
        2
    );
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let bytes = bincode::serialize(&saved).unwrap();
    let restored = restore_save(&registry, bincode::deserialize(&bytes).unwrap()).unwrap();
    assert_eq!(
        restored
            .finance()
            .get_account(source)
            .unwrap()
            .balance()
            .cents(),
        75_000
    );
    assert!(active_standstill(&restored, fixture.dominant, fixture.neighborhood).is_some());
}

#[test]
fn tribute_can_draw_from_multiple_spendable_cash_accounts_atomically() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::finance::finance_system::validate_record_transaction;
    use crate::finance::{LedgerPosting, LedgerTransactionDraft, Money};

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Rival Lieutenant",
    );
    for (name, organization, mandate, manager) in [
        (
            "Dominant Hall",
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
        ),
        (
            "Rival Hall",
            fixture.challenger,
            rival_mandate,
            rival_manager,
        ),
        (
            "Rival Annex",
            fixture.challenger,
            rival_mandate,
            rival_manager,
        ),
        (
            "Rival Outpost",
            fixture.challenger,
            rival_mandate,
            rival_manager,
        ),
    ] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(organization),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            organization,
            mandate,
            manager,
            venue,
        );
    }
    let first = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(fixture.dominant))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("an established racket has a street-cash account")
        .id();
    let second = org_cash(&mut fixture.state, fixture.dominant);
    let clearing = fresh_settlement(&mut fixture.state, fixture.dominant);
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund separate local cash pockets".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: first,
                    amount: Money::from_cents(10_000),
                },
                LedgerPosting {
                    account: second,
                    amount: Money::from_cents(15_000),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-25_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();

    let unchanged = bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(
        validate_negotiate_standstill_with_tribute(
            &fixture.state,
            fixture.dominant,
            fixture.challenger,
            fixture.neighborhood,
            Money::from_cents(25_001),
        )
        .err(),
        Some(TerritoryNegotiationError::InsufficientTributeFunds)
    );
    assert_eq!(
        bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap(),
        unchanged,
    );

    validate_negotiate_standstill_with_tribute(
        &fixture.state,
        fixture.dominant,
        fixture.challenger,
        fixture.neighborhood,
        Money::from_cents(25_000),
    )
    .expect("the combined spendable balance covers the required tribute")
    .commit(&mut fixture.state)
    .expect("the treaty and three-posting payment commit together");
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(first)
            .unwrap()
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(second)
            .unwrap()
            .balance(),
        Money::ZERO
    );
    let tribute_payment = fixture.state.finance().transactions().last().unwrap();
    assert_eq!(tribute_payment.postings().len(), 3);
    assert!(tribute_payment.postings().iter().any(|posting| {
        posting.account == first && posting.amount == Money::from_cents(-10_000)
    }));
    assert!(tribute_payment.postings().iter().any(|posting| {
        posting.account == second && posting.amount == Money::from_cents(-15_000)
    }));
    validate_invariants(&fixture.state);

    let saved = build_save(&registry, &fixture.state).unwrap();
    let restored = restore_save(
        &registry,
        bincode::deserialize(&bincode::serialize(&saved).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        restored.finance().get_account(first).unwrap().balance(),
        Money::ZERO
    );
    assert_eq!(
        restored.finance().get_account(second).unwrap().balance(),
        Money::ZERO
    );
    assert!(active_standstill(&restored, fixture.dominant, fixture.neighborhood).is_some());
}

#[test]
fn independent_wars_in_one_district_can_each_reach_autonomous_peace() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::world::territory_conflict::validate_territory_clash;

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let third = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Third Gang".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let fourth = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Fourth Gang".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let (second_manager, second_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Second Captain",
    );
    let (third_manager, third_mandate) = assign_district_authority(
        &mut fixture.state,
        third,
        fixture.neighborhood,
        "Third Captain",
    );
    let (fourth_manager, fourth_mandate) = assign_district_authority(
        &mut fixture.state,
        fourth,
        fixture.neighborhood,
        "Fourth Captain",
    );
    let mut rackets = Vec::new();
    for (organization, manager, mandate, name) in [
        (
            fixture.dominant,
            fixture.manager,
            fixture.mandate,
            "First Venue",
        ),
        (
            fixture.challenger,
            second_manager,
            second_mandate,
            "Second Venue",
        ),
        (third, third_manager, third_mandate, "Third Venue"),
        (fourth, fourth_manager, fourth_mandate, "Fourth Venue"),
    ] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(organization),
            hospitality_functions(),
        );
        rackets.push(establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            organization,
            mandate,
            manager,
            venue,
        ));
    }
    let day = SimDuration::from_minutes(u32::try_from(DAY_MINUTES).unwrap());
    fixture.state.advance_clock(day);
    for (attacker, target, expected_loser) in [
        (fixture.dominant, rackets[1], rackets[0]),
        (third, rackets[3], rackets[2]),
    ] {
        let result = validate_territory_clash(&fixture.state, attacker, target)
            .expect("independent gangs can fight on the same day")
            .commit(&mut fixture.state)
            .expect("independent clash commits");
        assert_eq!(result.suspended_racket, expected_loser);
    }
    assert!(apply_due_autonomous_territory_standstills(&mut fixture.state).is_empty());
    fixture.state.advance_clock(day);
    let agreements = apply_due_autonomous_territory_standstills(&mut fixture.state);
    assert_eq!(
        agreements.len(),
        2,
        "both unrelated defeated gangs should seek a separate ceasefire"
    );
    for (organization, counterparty) in [
        (fixture.dominant, fixture.challenger),
        (fixture.challenger, fixture.dominant),
        (third, fourth),
        (fourth, third),
    ] {
        assert_eq!(
            active_standstill(&fixture.state, organization, fixture.neighborhood)
                .expect("each gang must retain its own active peace")
                .counterparty,
            counterparty,
        );
    }
    assert!(apply_due_autonomous_territory_standstills(&mut fixture.state).is_empty());
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let restored = restore_save(&registry, saved).unwrap();
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}

#[test]
fn renewed_clash_restarts_the_peace_negotiation_wait() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::world::territory_conflict::validate_territory_clash;

    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (rival_manager, rival_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Second Captain",
    );
    let mut own = Vec::new();
    let mut rival = Vec::new();
    for (organization, manager, mandate, rackets) in [
        (fixture.dominant, fixture.manager, fixture.mandate, &mut own),
        (fixture.challenger, rival_manager, rival_mandate, &mut rival),
    ] {
        for index in 0..2 {
            let venue = insert_venue(
                &registry,
                &mut fixture.state,
                &format!("{organization} Hall {index}"),
                fixture.neighborhood,
                BusinessOwner::Organization(organization),
                hospitality_functions(),
            );
            rackets.push(establish_gambling_at_venue(
                &registry,
                &mut fixture.state,
                organization,
                mandate,
                manager,
                venue,
            ));
        }
    }
    let day = SimDuration::from_minutes(DAY_MINUTES as u32);
    fixture.state.advance_clock(day);
    let initial = validate_territory_clash(&fixture.state, fixture.dominant, rival[0])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(!initial.attacker_won);
    assert_eq!(initial.suspended_racket, own[0]);

    fixture.state.advance_clock(day);
    let renewed = validate_territory_clash(&fixture.state, fixture.challenger, own[1])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(renewed.attacker_won);
    assert_eq!(renewed.suspended_racket, own[1]);

    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(
        apply_due_autonomous_territory_standstills(&mut fixture.state).is_empty(),
        "an older clash must not permit negotiation on the minute of a renewed clash"
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut restored] {
        state.advance_clock(day);
        assert_eq!(apply_due_autonomous_territory_standstills(state).len(), 1);
        assert!(active_standstill(state, fixture.dominant, fixture.neighborhood).is_some());
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn autonomous_gangs_can_negotiate_a_real_postwar_standstill() {
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::{DAY_MINUTES, SimDuration};
    use crate::world::territory_conflict::apply_due_autonomous_territory_clashes;
    let registry = build_registry();
    let mut fixture = make_influence_fixture();
    let (defender_manager, defender_mandate) = assign_district_authority(
        &mut fixture.state,
        fixture.challenger,
        fixture.neighborhood,
        "Defending Lieutenant",
    );
    for name in ["First Offensive Hall", "Second Offensive Hall"] {
        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            name,
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.dominant),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            venue,
        );
    }
    let defended = insert_venue(
        &registry,
        &mut fixture.state,
        "Defended Hall",
        fixture.neighborhood,
        BusinessOwner::Organization(fixture.challenger),
        hospitality_functions(),
    );
    let displaced = establish_gambling_at_venue(
        &registry,
        &mut fixture.state,
        fixture.challenger,
        defender_mandate,
        defender_manager,
        defended,
    );
    let day = SimDuration::from_minutes(u32::try_from(DAY_MINUTES).unwrap());
    fixture.state.advance_clock(day);
    let clashes = apply_due_autonomous_territory_clashes(&mut fixture.state);
    assert_eq!(clashes.len(), 1);
    assert_eq!(clashes[0].suspended_racket, displaced);
    assert!(
        apply_due_autonomous_territory_standstills(&mut fixture.state).is_empty(),
        "peace negotiation cannot take place on the same minute as fighting"
    );
    fixture.state.advance_clock(day);
    let treaties = apply_due_autonomous_territory_standstills(&mut fixture.state);
    assert_eq!(
        treaties.len(),
        1,
        "the defeated NPC seeks peace on the next day"
    );
    for org in [fixture.dominant, fixture.challenger] {
        assert!(active_standstill(&fixture.state, org, fixture.neighborhood).is_some());
    }
    assert!(
        apply_due_autonomous_territory_standstills(&mut fixture.state).is_empty(),
        "a signed agreement cannot be issued twice"
    );
    validate_invariants(&fixture.state);
    let saved = build_save(&registry, &fixture.state).unwrap();
    let bytes = bincode::serialize(&saved).unwrap();
    let restored = restore_save(&registry, bincode::deserialize(&bytes).unwrap()).unwrap();
    assert!(active_standstill(&restored, fixture.challenger, fixture.neighborhood).is_some());
}
