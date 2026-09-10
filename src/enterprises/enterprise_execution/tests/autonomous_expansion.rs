//! Autonomous delegated-expansion behavior exercised through enterprise production paths.

use super::*;
use crate::world::territory_influence::resolve_neighborhood_influence;

fn designate_player(registry: &Registry, state: &mut AppState) -> OrganizationId {
    let player = insert_organization(
        registry,
        state,
        OrganizationDraft {
            name: "Player Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("player organization fixture should validate");
    crate::world::world_system::designate_player_organization(state, player)
        .expect("player designation fixture should validate");
    player
}

#[test]
fn autonomous_expansion_shared_slot_prefers_district_leader_over_organization_id() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };

    // The fixture organization has the lower stable id but no existing district influence.
    // A later-created rival owns one live racket here and therefore leads the district before
    // this autonomous pass. Stable creation order must not let the outsider claim the shared
    // Protection slot first.
    let leader_organization = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Higher Id District Leader".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("district leader organization should validate");
    assert!(leader_organization > fixture.organization);
    let leader_manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Higher Id District Manager".to_owned(),
            organization: Some(leader_organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("district leader manager should validate");
    // Give the incumbent only broad Enterprise authority while the lower-id outsider retains
    // its explicit neighborhood mandate. Internal delegation specificity must not outrank the
    // world-level fact that this organization already leads the contested district.
    let leader_scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    let leader_mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: leader_organization,
            manager: leader_manager,
            scopes: BTreeSet::from([leader_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("district leader mandate should validate")
    .commit(&mut fixture.state)
    .expect("district leader mandate should commit");
    let leader_cash = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(leader_organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("district leader cash account should validate");
    let leader_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(leader_organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("district leader settlement account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund higher-id district leader".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: leader_settlement,
                    amount: Money::from_cents(-100_000),
                },
                LedgerPosting {
                    account: leader_cash,
                    amount: Money::from_cents(100_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("district leader funding should validate")
    .commit(&mut fixture.state)
    .expect("district leader funding should commit");
    let leader_support = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Leader Hiring Warehouse".to_owned(),
            kind: BusinessKind::Warehouse,
            functions: BTreeSet::from([
                BusinessFunction::UnionAccess,
                BusinessFunction::Warehousing,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(leader_organization),
        },
    )
    .expect("district leader support business should validate");
    validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::LaborRacketeering,
            organization: leader_organization,
            authority: MandateAuthority {
                mandate: leader_mandate,
                manager: leader_manager,
                scope: leader_scope,
            },
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::from([leader_support]),
            cash_account: leader_cash,
            settlement_account: leader_settlement,
        },
    )
    .expect("leadership-establishing labor racket should validate")
    .commit(&mut fixture.state)
    .expect("leadership-establishing labor racket should commit");
    assert_eq!(
        resolve_neighborhood_influence(&fixture.state, neighborhood)
            .expect("district influence should resolve")
            .economic_leader(),
        Some(leader_organization)
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("competing autonomous expansion should resolve");
    let protection = fixture
        .state
        .enterprises()
        .enterprises_at(EnterpriseLocation::Neighborhood(neighborhood))
        .find(|enterprise| enterprise.kind() == EnterpriseKind::Protection)
        .expect("one contender should claim the shared protection slot");
    assert_eq!(
        protection.organization(),
        leader_organization,
        "phase-wide contention must honor district leadership before stable organization id"
    );
    assert!(established.contains(&protection.id()));
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_serves_governed_rivals_and_never_the_player_organization() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    let player = designate_player(&registry, &mut fixture.state);

    // The rival's mandate covers the district; the player organization has no mandate at all,
    // so even the designated player cannot receive autonomous establishments here.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "exactly one governed rival establishment per pass"
    );
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous enterprise should persist");
    assert_eq!(record.organization(), fixture.organization);
    assert_eq!(record.kind(), EnterpriseKind::Protection);
    assert_eq!(record.location(), fixture.location);
    // Asset-free kinds settle at the district itself through the covering scope.
    assert!(!matches!(
        record.location(),
        EnterpriseLocation::Business(_)
    ));
    validate_invariants(&fixture.state);
    let _ = player;
}

#[test]
fn autonomous_expansion_is_a_daily_cadence_gate() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("off-cadence autonomous expansion should resolve")
            .is_empty()
    );
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("due autonomous expansion should resolve")
            .len(),
        1,
        "the pass fires exactly on the day boundary"
    );
}

#[test]
fn autonomous_expansion_requires_one_current_cycle_of_working_capital() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let required = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        0,
    )
    .expect("fixture enterprise operating cost should fit");
    assert!(required > Money::ZERO);
    fund_enterprise_fixture_cash(&mut fixture, required.cents() - 1);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("undercapitalized autonomous expansion should resolve without mutation");
    assert!(
        established.is_empty(),
        "delegated expansion needs one current operating cycle of working capital"
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);

    fund_enterprise_fixture_cash(&mut fixture, 1);
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("fully capitalized autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_does_not_oracle_unobserved_district_case_pressure() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let base_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        0,
    )
    .expect("base protection runway should fit");
    let hidden_case_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        1,
    )
    .expect("heated protection runway should fit");
    assert!(
        hidden_case_runway > base_runway,
        "authored case pressure must materially change the actual operating runway"
    );
    fund_enterprise_fixture_cash(&mut fixture, base_runway.cents());
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Hidden Pressure Bureau",
        neighborhood,
    );
    let other_crew = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Other Market Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("other crew should validate");
    let other_scout = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Other Crew Scout".to_owned(),
            organization: Some(other_crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("other crew scout should validate");
    open_originated_pressure_case(
        &registry,
        &mut fixture,
        police,
        PressureCaseOrigin {
            organization: other_crew,
            manager: other_scout,
        },
        "Unreported district inquiry",
        EntityRef::Neighborhood(neighborhood),
        BTreeSet::from([other_crew]),
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .active_investigations()
            .filter(|investigation| investigation.owner() == police)
            .count(),
        1
    );
    assert!(
        fixture
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(fixture.organization))
            .all(|information| information.topic() != InformationTopic::LegalActivity),
        "the rival must not receive legal knowledge for the deliberately unreported case"
    );

    let remainder = u32::try_from(1_440_u64 - fixture.state.now().as_minutes())
        .expect("same-day remainder must fit simulation duration");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(remainder));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("unobserved pressure must not become autonomous planning foresight");
    assert_eq!(
        established.len(),
        1,
        "a delegated manager with no observed heat should plan from the known base runway"
    );
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous establishment should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::Protection);
    assert_eq!(enterprise.location(), fixture.location);
    validate_state(&fixture.state).expect("non-oracle autonomous expansion should stay valid");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_discards_stale_observed_district_pressure() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let base_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        0,
    )
    .expect("base protection runway should fit");
    let heated_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        1,
    )
    .expect("heated protection runway should fit");
    assert!(heated_runway > base_runway);

    let retired = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Historical Pressure Bureau",
        neighborhood,
    );
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Historical district inquiry",
        neighborhood,
    );
    settle_cycle_inner(&registry, &mut fixture, retired);
    let observed = fixture
        .state
        .enterprises()
        .latest_cycle(retired)
        .expect("heated cycle should persist");
    assert_eq!(
        observed.investigation_heat(),
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .heat_surcharge_per_active_case(),
        "the organization must first have a real settled heat observation"
    );
    validate_suspend_enterprise(&fixture.state, retired)
        .expect("heated enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("heated enterprise suspension should commit");
    validate_retire_enterprise(&fixture.state, retired)
        .expect("suspended enterprise should retire")
        .commit(&mut fixture.state)
        .expect("heated enterprise retirement should commit");

    // Save/restore deliberately drops derived indexes. Rebuild must recover the recent-cycle
    // time projection from authoritative history or the observation below would disappear early.
    fixture.state = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("retired heated enterprise should remain saveable"),
    )
    .expect("retired heated enterprise should restore with derived cycle indexes rebuilt");

    // Normalize liquid cash to exactly the quiet runway. If the retired racket's last observed
    // heat were remembered forever, that stale surcharge would make the replacement appear
    // unaffordable and permanently block re-entry.
    let cash_balance = fixture
        .state
        .finance()
        .get_account(fixture.cash)
        .expect("fixture cash should persist")
        .balance();
    let adjustment = base_runway
        .checked_sub(cash_balance)
        .expect("fixture cash normalization should fit");
    if adjustment != Money::ZERO {
        validate_record_transaction(
            &fixture.state,
            LedgerTransactionDraft {
                occurred_at: fixture.state.now(),
                memo: "Normalize stale-pressure regression runway".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: fixture.cash,
                        amount: adjustment,
                    },
                    LedgerPosting {
                        account: fixture.settlement,
                        amount: adjustment
                            .checked_neg()
                            .expect("runway adjustment should negate"),
                    },
                ],
                authorization: None,
            },
        )
        .expect("cash normalization should validate")
        .commit(&mut fixture.state)
        .expect("cash normalization should commit");
    }

    let mut still_fresh = fixture.state.clone();
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut still_fresh)
            .expect("restored recent pressure should remain usable planning knowledge")
            .is_empty(),
        "the rebuilt recent-cycle index must retain fresh observed heat after restore"
    );

    let cold_window = registry.legal().cold_case_window();
    fixture.state.advance_clock(cold_window);
    apply_cold_case_decay(&mut fixture.state, cold_window)
        .expect("historical originated case should decay");
    let minute_in_day = fixture.state.now().as_minutes() % crate::core::time::DAY_MINUTES;
    if minute_in_day != 0 {
        fixture.state.advance_clock(SimDuration::from_minutes(
            u32::try_from(crate::core::time::DAY_MINUTES - minute_in_day)
                .expect("same-day boundary remainder must fit duration"),
        ));
    }

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("stale observed heat should not block later autonomous re-entry");
    assert_eq!(established.len(), 1);
    let replacement = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("replacement enterprise should persist");
    assert_eq!(replacement.kind(), EnterpriseKind::Protection);
    assert_eq!(replacement.location(), fixture.location);
    validate_state(&fixture.state).expect("stale-pressure recovery state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_skips_unaffordable_candidate_for_best_affordable_kind() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let protection = establish_protection(&registry, &mut fixture);
    let protection_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        0,
    )
    .expect("existing protection runway should fit");
    fund_enterprise_fixture_cash(
        &mut fixture,
        protection_runway
            .cents()
            .checked_add(6_200)
            .expect("fixture funding should fit"),
    );

    // This venue makes Gambling a valid high-return candidate, but its current-cycle runway is
    // above the $62 uncommitted treasury after reserving the existing Protection runway.
    // LoanSharking is both affordable and the strongest zero-variance net among the affordable
    // venue-backed choices, so financing-aware ranking must continue instead of abandoning the
    // pass or falling back to enum order.
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Lean Rival Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("financing-aware autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(established[0])
            .expect("selected enterprise should persist")
            .kind(),
        EnterpriseKind::LoanSharking
    );
    assert!(
        fixture
            .state
            .enterprises()
            .get_enterprise(protection)
            .is_some()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_does_not_spend_existing_racket_runway_twice() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 6_200);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Runway Guard Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("working-capital reservation pass should resolve");
    assert!(
        established.is_empty(),
        "cash already backing an active racket cannot simultaneously fund another runway"
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 1);
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_can_reenter_a_slot_released_by_retirement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    let retired = establish_protection(&registry, &mut fixture);
    validate_suspend_enterprise(&fixture.state, retired)
        .expect("active enterprise should suspend before retirement")
        .commit(&mut fixture.state)
        .expect("enterprise suspension should commit");
    crate::enterprises::enterprise_execution::validate_retire_enterprise(&fixture.state, retired)
        .expect("suspended enterprise should retire")
        .commit(&mut fixture.state)
        .expect("enterprise retirement should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("retired history should not block autonomous re-entry");
    assert_eq!(established.len(), 1);
    let replacement = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("replacement enterprise should persist");
    assert_eq!(replacement.kind(), EnterpriseKind::Protection);
    assert_eq!(replacement.location(), fixture.location);
    assert_ne!(replacement.id(), retired);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(retired)
            .expect("retired enterprise remains durable history")
            .status(),
        EnterpriseStatus::Retired
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_allocates_scarce_runway_to_stronger_same_day_mandate() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let one_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
        0,
    )
    .expect("protection runway should fit");

    let second_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Stronger Governed Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(100),
                    commercial_activity: rating(100),
                    illicit_demand: rating(100),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(0),
                },
            },
        },
    )
    .expect("second neighborhood should validate");
    let second_runway = resolve_enterprise_operating_cost_projection(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        EnterpriseLocation::Neighborhood(second_neighborhood),
        0,
        0,
    )
    .expect("second protection runway should fit");
    fund_enterprise_fixture_cash(
        &mut fixture,
        one_runway
            .cents()
            .checked_add(second_runway.cents())
            .and_then(|cents| cents.checked_sub(1))
            .expect("fixture funding should fit"),
    );
    let second_manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second Enterprise Manager".to_owned(),
            organization: Some(fixture.organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second manager should validate");
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager: second_manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(second_neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("second mandate should validate")
    .commit(&mut fixture.state)
    .expect("second mandate should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("multi-mandate autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "one cash pool that cannot cover two runways must not create two same-day rackets"
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(established[0])
            .expect("selected establishment should persist")
            .location(),
        EnterpriseLocation::Neighborhood(second_neighborhood),
        "shared treasury must fund the stronger current opportunity instead of the earlier-created mandate"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_assembles_authored_multi_business_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);

    // Alcohol distribution is authored as a district racket backed by a network, not a venue:
    // transport/storage/distribution can come from one owned business while customer access
    // comes from another. The autonomous planner must compose the same support shape accepted by
    // the canonical establishment path instead of looking for an impossible all-in-one host.
    let organization = fixture.organization;
    let transport = insert_support_business(
        &registry,
        &mut fixture,
        "Autonomous Freight Network",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let retail = insert_support_business(
        &registry,
        &mut fixture,
        "Autonomous Bottle Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("complete authored network should support autonomous expansion");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(enterprise.location(), fixture.location);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([transport, retail])
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_prefers_support_that_covers_more_unmet_network_functions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Insert partial businesses first so raw ID order alone would choose redundant support.
    // A later integrated depot covers every authored AlcoholDistribution network function and
    // should therefore be selected alone, avoiding the per-support operating surcharge.
    insert_support_business(
        &registry,
        &mut fixture,
        "Partial Freight Yard",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
        ]),
        BusinessOwner::Organization(organization),
    );
    insert_support_business(
        &registry,
        &mut fixture,
        "Partial Retail Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );
    let integrated = insert_support_business(
        &registry,
        &mut fixture,
        "Integrated Distribution Depot",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("integrated support network should support autonomous expansion");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([integrated]),
        "support planning should not attach redundant surcharge-producing businesses"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_finds_minimum_support_cover_when_greedy_would_overpay() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Required AlcoholDistribution network functions are vehicle, warehouse, distribution,
    // customer access. A greedy ID-first maximum-coverage choice would take `first` ({V,W}),
    // then need both later businesses. The exact cover is the latter two businesses only.
    let first = insert_support_business(
        &registry,
        &mut fixture,
        "Greedy Trap Storage",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
        ]),
        BusinessOwner::Organization(organization),
    );
    let distribution = insert_support_business(
        &registry,
        &mut fixture,
        "Distribution Link",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let retail = insert_support_business(
        &registry,
        &mut fixture,
        "Warehouse Retail Link",
        BusinessKind::Retail,
        BTreeSet::from([
            BusinessFunction::Warehousing,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("minimum-cover autonomous expansion should resolve");
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([distribution, retail])
    );
    assert!(!enterprise.supporting_businesses().contains(&first));
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_chooses_cheaper_host_within_same_district_authority() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Both clubs satisfy the Speakeasy venue requirements. The lower-ID club contributes none
    // of the supply-chain network, so it would need both the higher-ID club and the brewery as
    // supports. The higher-ID club contributes distribution itself and needs only the brewery.
    // District authority is identical, therefore the manager should avoid the extra $75/cycle
    // authored support surcharge instead of blindly taking the first business ID.
    let _expensive_host = insert_support_business(
        &registry,
        &mut fixture,
        "Early Nightlife Club",
        BusinessKind::Nightclub,
        BTreeSet::from([
            BusinessFunction::Nightlife,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let cheaper_host = insert_support_business(
        &registry,
        &mut fixture,
        "Integrated Nightlife Club",
        BusinessKind::Nightclub,
        BTreeSet::from([
            BusinessFunction::Nightlife,
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let brewery = insert_support_business(
        &registry,
        &mut fixture,
        "Supply Brewery",
        BusinessKind::Brewery,
        BTreeSet::from([BusinessFunction::AlcoholProduction]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("cheaper hosted expansion should resolve");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous hosted enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::Speakeasy);
    assert_eq!(
        enterprise.location(),
        EnterpriseLocation::Business(cheaper_host)
    );
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([brewery])
    );
    validate_invariants(&fixture.state);
}

#[test]
fn payroll_can_use_organization_cash_referenced_by_an_enterprise() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let outcome =
        crate::world::payroll_execution::apply_daily_payroll(&registry, &mut fixture.state)
            .expect("payroll should settle from organization-owned liquid cash")
            .into_iter()
            .find(|outcome| outcome.organization() == fixture.organization)
            .expect("the staffed organization should run payroll");
    assert_eq!(outcome.paid(), outcome.owed());
    assert_eq!(outcome.short(), Money::ZERO);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("enterprise cash account should persist")
            .balance(),
        Money::from_cents(10_000)
            .checked_sub(outcome.owed())
            .expect("funded fixture can cover one member's payroll")
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_surfaces_enterprise_id_exhaustion_without_partial_establishment() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let enterprise_count_before = fixture.state.enterprises().enterprises().count();
    let account_count_before = fixture.state.finance().accounts().count();
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Enterprise, u32::MAX);

    let error = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect_err("autonomous expansion must surface enterprise allocator exhaustion");
    assert!(matches!(
        error,
        crate::enterprises::autonomous_expansion::AutonomousExpansionError::Enterprise(
            EnterpriseError::IdExhaustion(_)
        )
    ));
    assert_eq!(
        fixture.state.enterprises().enterprises().count(),
        enterprise_count_before,
        "failed expansion must not insert an enterprise"
    );
    assert_eq!(
        fixture.state.finance().accounts().count(),
        account_count_before,
        "failed expansion must not consume or open finance records"
    );
    validate_state(&fixture.state).expect("failed autonomous expansion must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_rotates_kinds_and_hosts_the_rival_venue() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 20_000);

    // An owned hospitality venue inside the governed district can host every
    // cash-and-space racket kind.
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Rival Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-one autonomous expansion should resolve");
    assert_eq!(first_day.len(), 1);
    let first_kind = {
        let first = fixture
            .state
            .enterprises()
            .get_enterprise(first_day[0])
            .expect("day-one enterprise should persist");
        (first.kind(), first.location(), first.settlement_account())
    };
    assert_eq!(
        first_kind.0,
        EnterpriseKind::LoanSharking,
        "the strongest current zero-variance net should outrank incidental enum order"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-two autonomous expansion should resolve");
    assert_eq!(second_day.len(), 1);
    // The strongest remaining profitable configuration is gambling at the same venue. A second
    // loan-sharking record at the occupied location is not a valid candidate.
    let second_kind = {
        let second_probe = fixture
            .state
            .enterprises()
            .get_enterprise(second_day[0])
            .expect("day-two enterprise should persist");
        (second_probe.kind(), second_probe.location())
    };
    assert_eq!(second_kind.0, EnterpriseKind::Gambling);
    assert!(matches!(second_kind.1, EnterpriseLocation::Business(_)));
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let third_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-three autonomous expansion should resolve");
    assert_eq!(third_day.len(), 1);
    let third = fixture
        .state
        .enterprises()
        .get_enterprise(third_day[0])
        .expect("day-three enterprise should persist");
    assert_eq!(third.kind(), EnterpriseKind::Bookmaking);
    let EnterpriseLocation::Business(host) = third.location() else {
        panic!(
            "bookmaking must host at the venue, got {:?}",
            third.location()
        );
    };
    let host_name = fixture
        .state
        .world()
        .get_business(host)
        .expect("hosted venue should exist")
        .name()
        .to_owned();
    assert_eq!(host_name, "Rival Card Room");

    // Each establishment reserved its own exclusive settlement account.
    assert_ne!(first_kind.2, third.settlement_account());
    validate_invariants(&fixture.state);
}

#[test]
fn same_tick_vice_fear_blocks_due_autonomous_expansion() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 1_000_000);
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let expansion_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Clean Expansion Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(20),
                },
            },
        },
    )
    .expect("clean expansion district should validate");
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(neighborhood),
                ResponsibilityScope::Neighborhood(expansion_neighborhood),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("two-district mandate should validate")
    .commit(&mut fixture.state)
    .expect("two-district mandate should commit");
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Same-Tick Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Same-Tick Vice Bureau",
        neighborhood,
    );

    // Enough independent district-pressure cases make this enterprise's due vice roll certain.
    // They deliberately target the neighborhood rather than the enterprise so they create heat
    // without already counting as the dedicated inquiry the cycle should draw.
    let per_case = u32::from(
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .vice_attention_basis_points_per_active_case(),
    );
    assert!(
        per_case > 0,
        "the authored protection racket must carry vice risk"
    );
    let pressure_case_count = 10_000_u32.div_ceil(per_case);
    for index in 0..pressure_case_count {
        validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: police,
                title: format!("Same-tick district pressure {index}"),
                subjects: BTreeSet::from([EntityRef::Neighborhood(neighborhood)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Neighborhood(neighborhood),
                    origin: Some(EntityRef::Enterprise(enterprise)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin: Some(EntityRef::Enterprise(enterprise)),
                notified_organizations: BTreeSet::from([fixture.organization]),
                witness: None,
            },
        )
        .expect("district-pressure intake should validate")
        .commit(&mut fixture.state)
        .expect("district-pressure intake should commit");
    }

    // Start one point below the value that will become the ceiling after day-boundary decay
    // followed by the authored vice consequence: 45 -> 44 decay -> +6 vice = 50.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        5,
    )
    .expect("pre-tick fear setup should apply");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    // Prove the organization really would expand at this boundary if it read the stale
    // pre-consequence posture. The control intentionally does not assert a destination: delegated
    // planning may use only pressure the organization has actually observed, while these synthetic
    // district cases exist only to force the same-tick vice consequence below.
    let mut stale_posture_control = fixture.state.clone();
    stale_posture_control.advance_clock(SimDuration::ONE_MINUTE);
    let stale_expansion = apply_due_autonomous_enterprises(&registry, &mut stale_posture_control)
        .expect("pre-consequence posture should support expansion");
    assert_eq!(
        stale_expansion.len(),
        1,
        "the regression requires a genuinely eligible expansion under the old posture"
    );
    assert!(
        stale_posture_control
            .enterprises()
            .get_enterprise(stale_expansion[0])
            .is_some(),
        "control expansion should persist"
    );

    let outcome = run_tick(&registry, &mut fixture.state);
    assert_eq!(outcome.enterprise_cycles.len(), 1);
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(outcome.enterprise_cycles[0])
            .expect("due enterprise cycle should persist")
            .drew_vice_attention(),
        "certainty-level district pressure must draw the same-tick vice inquiry"
    );
    assert_eq!(
        crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        ),
        registry.reputation().expansion_police_fear_ceiling(),
        "day-boundary decay plus the fresh vice consequence should land exactly on the ceiling"
    );
    assert!(
        outcome.autonomous_enterprises.is_empty(),
        "same-minute vice fear must reach the expansion gate before delegated growth runs"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn police_fear_at_or_above_the_authored_ceiling_stalls_expansion_until_it_cools() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    let ceiling = registry.reputation().expansion_police_fear_ceiling();

    // Drive the outfit visibly hot through the canonical reputation path.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        100,
    )
    .expect("fear adjustment should apply");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("hot-posture autonomous expansion should resolve")
            .is_empty(),
        "an outfit at or above the fear ceiling must keep its head down"
    );

    // Exactly the authored ceiling is still hot, not the first safe score.
    loop {
        let fear = crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        );
        if fear <= ceiling {
            break;
        }
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        crate::reputation::reputation_system::apply_daily_reputation_decay(
            &registry,
            &mut fixture.state,
        );
    }
    assert_eq!(
        crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        ),
        ceiling
    );
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("ceiling-posture autonomous expansion should resolve")
            .is_empty(),
        "the authored ceiling itself must keep delegated expansion paused"
    );

    // Once the impression decays below the ceiling the same mandate expands again.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        -1,
    )
    .expect("cooling adjustment should apply");
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "cooled-down outfits resume governed expansion"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn expansion_consolidates_led_districts_before_contested_ones() {
    use crate::enterprises::EnterpriseDraft;

    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    // Capital is deliberately non-binding in this influence-ordering test. Existing active
    // rackets reserve their own current-cycle runway before delegated expansion considers a new
    // one, so a token balance would make this a financing test instead of an ordering test.
    fund_enterprise_fixture_cash(&mut fixture, 100_000);

    // Fixture intent: the LED district carries the HIGHER id. Selection that followed raw
    // id order would open in the un-led district first; influence-aware preference must
    // consolidate the led one instead.
    let contested = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use district locations"),
    };
    let led = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Led Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(55),
                    illicit_demand: rating(45),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("led neighborhood should validate");
    assert!(led > contested);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(contested),
                ResponsibilityScope::Neighborhood(led),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate revision should validate")
    .commit(&mut fixture.state)
    .expect("mandate revision should commit");

    // Leadership of the higher-id district: an owned venue hosting a gambling racket.
    let organization = fixture.organization;
    let led_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Led Ward Card Room".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood: led,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("led venue should validate");
    let settlement = crate::finance::finance_system::insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: MandateAuthority {
                mandate: fixture.authority.mandate,
                manager: fixture.authority.manager,
                scope: ResponsibilityScope::Neighborhood(led),
            },
            location: EnterpriseLocation::Business(led_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: settlement,
        },
    )
    .expect("leadership enterprise should validate")
    .commit(&mut fixture.state)
    .expect("leadership enterprise should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    let established_record = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("establishment should persist");
    assert_eq!(
        established_record.authority().scope,
        ResponsibilityScope::Neighborhood(led),
        "consolidation preference must choose authority in the led district before contested territory"
    );
    let selected_neighborhood = match established_record.location() {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(business) => fixture
            .state
            .world()
            .get_business(business)
            .expect("selected host business must persist")
            .neighborhood(),
    };
    assert_eq!(
        selected_neighborhood, led,
        "economics may choose either a district racket or a hosted racket, but it must remain inside the led district"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn expansion_uses_economics_within_the_same_influence_tier() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);

    let lower_id_district = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use district locations"),
    };
    let richer_higher_id_district = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Prosperous Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(100),
                    commercial_activity: rating(100),
                    illicit_demand: rating(100),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(0),
                },
            },
        },
    )
    .expect("richer neighborhood should validate");
    assert!(richer_higher_id_district > lower_id_district);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(lower_id_district),
                ResponsibilityScope::Neighborhood(richer_higher_id_district),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("two-district mandate revision should validate")
    .commit(&mut fixture.state)
    .expect("two-district mandate revision should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("same-tier economic ranking should resolve");
    assert_eq!(established.len(), 1);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(established[0])
            .expect("same-tier expansion should persist")
            .location(),
        EnterpriseLocation::Neighborhood(richer_higher_id_district),
        "district ID must not outrank stronger current economics inside one influence tier"
    );
    validate_invariants(&fixture.state);
}
