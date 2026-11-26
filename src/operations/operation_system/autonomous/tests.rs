use super::*;
use crate::city_generation::{CityScale, generate_city};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_test_tick;
use crate::delegation::MandateDraft;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::finance::finance_system::insert_account;
use crate::finance::finance_system::validate_record_transaction;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::finance::{LedgerPosting, LedgerTransactionDraft, Money};
use crate::politics::political_system::validate_invest_influence;
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crate::world::{
    BusinessDraft, BusinessFunction, BusinessOwner, CapabilityKind, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, Rating,
};

struct Fixture {
    registry: Registry,
    state: AppState,
    rival: OrganizationId,
    player: OrganizationId,
    competitor: BusinessId,
    manager: CharacterId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).unwrap()
}

fn fixture() -> Fixture {
    fixture_with_scout_rating(100)
}

fn fixture_with_scout_rating(scout_rating: u8) -> Fixture {
    fixture_with_competitors(scout_rating, None)
}

fn fixture_with_competitors(scout_rating: u8, second_competitor: Option<BusinessKind>) -> Fixture {
    let registry = crate::build_registry();
    let mut state = AppState::new(0x95AC);
    let mut organization = |name: &str| {
        insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: name.to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap()
    };
    let rival = organization("Harbor Crew");
    let player = organization("Finch Holdings");
    designate_player_organization(&mut state, player).unwrap();

    let mut district = |name: &str| {
        insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: name.to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(60),
                        commercial_activity: rating(60),
                        illicit_demand: rating(55),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(0),
                    },
                },
            },
        )
        .unwrap()
    };
    let harbor = district("Harbor");
    let elsewhere = district("Uptown");
    let mut venue = |name: &str, owner, neighborhood, kind| {
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: name.to_owned(),
                kind,
                functions: BTreeSet::from([
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::ProfessionalRecords,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(owner),
            },
        )
        .unwrap();
        let operating_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .unwrap();
        let settlement_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_establish_business_economy(
            &registry,
            &state,
            BusinessEconomyDraft {
                business,
                operating_account,
                settlement_account,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        business
    };
    // The lower-ID foreign property is out of territory and must not be selected.
    venue("Uptown Store", player, elsewhere, BusinessKind::Retail);
    venue("Harbor Crew Store", rival, harbor, BusinessKind::Retail);
    let competitor = venue("Finch Harbor Store", player, harbor, BusinessKind::Retail);
    if let Some(kind) = second_competitor {
        venue("Second Finch Harbor Store", player, harbor, kind);
    }

    let mut member = |organization, name: &str, capabilities| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities,
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap()
    };
    let manager = member(
        rival,
        "Harbor Leader",
        BTreeMap::from([(CapabilityKind::Management, rating(100))]),
    );
    member(
        rival,
        "Harbor Scout",
        BTreeMap::from([
            (CapabilityKind::Surveillance, rating(scout_rating)),
            (CapabilityKind::Burglary, rating(100)),
            (CapabilityKind::Stealth, rating(100)),
        ]),
    );
    let player_manager = member(
        player,
        "Finch Leader",
        BTreeMap::from([
            (CapabilityKind::Management, rating(100)),
            (CapabilityKind::Surveillance, rating(100)),
        ]),
    );
    for (organization, manager) in [(rival, manager), (player, player_manager)] {
        validate_assign_mandate(
            &state,
            MandateDraft {
                organization,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Operations,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
    }
    Fixture {
        registry,
        state,
        rival,
        player,
        competitor,
        manager,
    }
}

fn next_day(registry: &Registry, state: &mut AppState) -> crate::core::simulation::TickOutcome {
    let remaining = DAY_MINUTES - state.now().as_minutes() % DAY_MINUTES;
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(remaining - 1).unwrap(),
    ));
    run_test_tick(registry, state)
}

fn finish_operation(registry: &Registry, state: &mut AppState, id: crate::core::id::OperationId) {
    // Execute the real tick, not a fabricated operation result or economy mutation.
    for _ in 0..400 {
        run_test_tick(registry, state);
        if !ACTIVE_ASSIGNMENT_STATUSES
            .contains(&state.operations().get_operation(id).unwrap().status())
        {
            return;
        }
    }
    panic!("operation {id} did not terminate");
}

#[test]
fn rival_market_forecast_accounts_for_damage_to_its_own_stake_not_foreign_damage() {
    use crate::economy::business_economy_system::{
        resolve_business_current_gross, validate_disrupt_business_economy,
    };

    let Fixture {
        registry,
        mut state,
        rival,
        competitor,
        ..
    } = fixture();
    let stake = state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .id();
    let healthy_gain =
        forecast_competitor_disruption_gain(&registry, &state, competitor, &[stake]).unwrap();
    validate_disrupt_business_economy(&registry, &state, stake)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let before = resolve_business_current_gross(&registry, &state, stake).unwrap();
    let forecast =
        forecast_competitor_disruption_gain(&registry, &state, competitor, &[stake]).unwrap();

    // With only two local shops, the public healthy-competitor assumption is true
    // before the hypothetical attack. Compare with the actual economy after it.
    validate_disrupt_business_economy(&registry, &state, competitor)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let after = resolve_business_current_gross(&registry, &state, stake).unwrap();
    let cycle = registry
        .get_business(BusinessKind::Retail)
        .economics()
        .cycle();
    let expected = i128::from(after.cents() - before.cents()) * i128::from(DAY_MINUTES)
        / i128::from(cycle.as_minutes());
    assert_eq!(forecast, expected);
    assert!(forecast > 0 && forecast < healthy_gain);
    assert_eq!(
        forecast_competitor_disruption_gain(&registry, &state, competitor, &[stake]).unwrap(),
        forecast,
        "private foreign disruption cannot change the planner's forecast"
    );
}

#[test]
fn refused_protection_prioritizes_the_refusing_owner_not_other_ward_businesses() {
    use crate::decisions::DecisionResponse;
    use crate::decisions::decision_system::{
        apply_due_protection_demands, validate_resolve_decision,
    };
    use crate::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};
    use crate::enterprises::enterprise_execution::validate_establish_enterprise;
    use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};

    let Fixture {
        registry,
        mut state,
        rival,
        player,
        competitor,
        manager,
    } = fixture();
    let district = state
        .world()
        .get_business(competitor)
        .unwrap()
        .neighborhood();
    let third_party = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Uninvolved Holdings".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let third_party_shop = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Uninvolved Nightclub".to_owned(),
            kind: BusinessKind::Nightclub,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood: district,
            owner: BusinessOwner::Organization(third_party),
        },
    )
    .unwrap();
    let operating_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(third_party_shop),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(third_party_shop),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business: third_party_shop,
            operating_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();

    let mandate = state.delegation().active_for_manager(manager).unwrap().id();
    let scope = ResponsibilityScope::Neighborhood(district);
    validate_revise_mandate(
        &state,
        mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                scope,
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let cash_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: rival,
            authority: MandateAuthority {
                mandate,
                manager,
                scope,
            },
            location: EnterpriseLocation::Neighborhood(district),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));

    let mut control = state.clone();
    let targets = |state: &mut AppState| {
        apply_due_rival_operations(&registry, state)
            .into_iter()
            .map(|id| {
                state
                    .operations()
                    .get_operation(id)
                    .unwrap()
                    .objective()
                    .business_target()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        targets(&mut control),
        vec![Some(third_party_shop)],
        "without a refusal, the wealthier target wins"
    );
    let demands = apply_due_protection_demands(&registry, &mut state);
    assert_eq!(demands.len(), 1);
    validate_resolve_decision(
        &registry,
        &state,
        demands[0].decision,
        player,
        DecisionResponse::Reject,
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    assert_eq!(
        targets(&mut state),
        vec![Some(competitor)],
        "retaliation belongs to the refusing owner, not an uninvolved neighbor"
    );
}

#[test]
fn rival_scouts_local_competitor_then_disrupts_actual_player_business_after_restore() {
    let Fixture {
        registry,
        mut state,
        rival,
        player,
        competitor,
        ..
    } = fixture();
    let tick = next_day(&registry, &mut state);
    assert_eq!(
        tick.autonomous_operations.len(),
        1,
        "player mandates must not authorize NPC moves"
    );
    let scout = tick.autonomous_operations[0];
    let record = state.operations().get_operation(scout).unwrap();
    assert_eq!(record.kind(), OperationKind::Surveillance);
    assert_eq!(record.responsible_organization(), rival);
    assert_eq!(record.objective().business_target(), Some(competitor));
    assert!(
        state
            .operations()
            .operations_for_organization(player)
            .next()
            .is_none()
    );
    assert!(
        apply_due_rival_operations(&registry, &mut state).is_empty(),
        "no second plan with active work"
    );
    finish_operation(&registry, &mut state, scout);
    let access = state
        .intelligence()
        .information_for_holder_subject(
            KnowledgeHolder::Organization(rival),
            EntityRef::Business(competitor),
        )
        .find(|info| {
            info.topic() == InformationTopic::MarketAccess
                && info.source_kind() == InformationSourceKind::Surveillance
        })
        .expect("successful reconnaissance discovers venue access")
        .id();

    let mut control = state.clone();
    let manager = control
        .world()
        .characters_in_organization(rival)
        .find(|person| {
            control
                .delegation()
                .active_for_manager(person.id())
                .is_some()
        })
        .unwrap()
        .id();
    let mandate = control
        .delegation()
        .active_for_manager(manager)
        .unwrap()
        .id();
    // The same authentic venue-access observation is applicable to the other
    // authored property-disruption kind, without enabling autonomous arson.
    let mut explicit_arson = state.clone();
    let objective = OperationObjective::DisruptBusiness {
        target: EntityRef::Business(competitor),
    };
    let scheduled_for = state.now() + SimDuration::ONE_MINUTE;
    let specialist = state.operations().get_operation(scout).unwrap().leader();
    let arson = validate_authorize_operation(
        &registry,
        &explicit_arson,
        OperationDraft {
            title: "Explicit venue disruption".to_owned(),
            kind: OperationKind::Arson,
            responsible_organization: rival,
            leader: manager,
            objective,
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, manager),
                (RoleKind::EntrySpecialist, specialist),
            ]),
            intelligence: BTreeSet::from([access]),
            constraints: vec![
                crate::operations::OperationConstraint::RequireIntelligenceTopic(
                    InformationTopic::MarketAccess,
                ),
            ],
            contingencies: Vec::new(),
            scheduled_for,
        },
    )
    .unwrap()
    .commit(&mut explicit_arson)
    .unwrap();
    assert!(
        run_test_tick(&registry, &mut explicit_arson)
            .started_operations
            .contains(&arson)
    );
    restore_save(&registry, build_save(&registry, &explicit_arson).unwrap()).unwrap();
    crate::delegation::delegation_system::validate_revoke_mandate(&control, mandate)
        .unwrap()
        .commit(&mut control)
        .unwrap();
    let rival_shop = state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .id();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut state);
    let replay = next_day(&registry, &mut restored);
    assert_eq!(tick, replay);
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    assert_eq!(tick.autonomous_operations.len(), 1);
    let strike = tick.autonomous_operations[0];
    let record = state.operations().get_operation(strike).unwrap();
    assert_eq!(record.kind(), OperationKind::Sabotage);
    assert_eq!(record.objective().business_target(), Some(competitor));
    assert_eq!(record.responsible_organization(), rival);
    assert!(
        record.intelligence().contains(&access),
        "the hostile operation must actually use the access observation that justified it"
    );
    finish_operation(&registry, &mut state, strike);
    assert!(
        state
            .operations()
            .get_operation(strike)
            .unwrap()
            .resolution()
            .unwrap()
            .factors()
            .intelligence_topics_covered()
            >= 2,
        "venue access and the observed patrol pattern both contribute to the operation"
    );
    assert!(
        state
            .economy()
            .get_business_economy(competitor)
            .unwrap()
            .is_disrupted(state.now())
    );
    assert!(state.reports().reports_for(player).any(|report| {
        report
            .title()
            .contains("Hostile attack on Finch Harbor Store")
            && report.entries().iter().any(|entry| {
                entry.attention == crate::core::attention::AttentionClass::Crisis
                    && entry.entities.contains(&EntityRef::Business(competitor))
            })
    }));
    let settlement = next_day(&registry, &mut state);
    next_day(&registry, &mut control);
    next_day(&registry, &mut control);
    let rival_cycle = state.economy().latest_cycle(rival_shop).unwrap();
    let control_cycle = control.economy().latest_cycle(rival_shop).unwrap();
    assert_eq!(rival_cycle.occurred_at(), control_cycle.occurred_at());
    assert_eq!(
        rival_cycle.variance_basis_points(),
        control_cycle.variance_basis_points()
    );
    assert!(
        rival_cycle.gross_revenue() > control_cycle.gross_revenue(),
        "successful pressure must improve the attacker's own operating revenue"
    );
    assert!(
        rival_cycle.market_gross_basis_points(&registry)
            > control_cycle.market_gross_basis_points(&registry)
    );
    assert!(
        state
            .economy()
            .latest_cycle(competitor)
            .unwrap()
            .gross_revenue()
            < control
                .economy()
                .latest_cycle(competitor)
                .unwrap()
                .gross_revenue()
    );
    assert!(settlement.business_cycles.iter().any(|cycle| {
        let cycle = state.economy().get_cycle(*cycle).unwrap();
        cycle.business() == competitor
            && cycle.disrupted()
            && cycle.attention() == crate::core::attention::AttentionClass::Notable
    }));
    crate::core::invariants::validate_state(&state).unwrap();
    restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
}

#[test]
fn abandoned_rackets_expose_clean_stores_to_gangs_without_local_commercial_competition() {
    use crate::enterprises::enterprise_execution::{
        validate_staff_enterprise, validate_suspend_enterprise,
    };

    let registry = crate::build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(82_734_113)).unwrap();
    let rival = city.rivals[0];
    let player = city.player;
    let target = city.starter_front;
    let rival_racket = city
        .state
        .enterprises()
        .active_for_organization(rival)
        .next()
        .unwrap();
    let staffed = rival_racket.kind().max_associates();
    let racket_id = rival_racket.id();
    let rival_district = rival_racket
        .location()
        .neighborhood(city.state.world())
        .unwrap();
    let strongest_force = u32::from(rival_racket.associates());
    let force_by_district = BTreeMap::from([(rival_district, strongest_force)]);
    assert_eq!(
        predatory_stake(
            &registry,
            &city.state,
            rival,
            target,
            &force_by_district,
            strongest_force,
            &mut BTreeMap::new(),
        ),
        0,
        "an operating home racket keeps its affiliated shops out of predatory reach"
    );
    // A genuine opposing workforce can defeat the named enforcers who
    // remain after the player's local racket closes. No virtual attack
    // strength is injected into the operation planner.
    validate_staff_enterprise(&city.state, racket_id, staffed)
        .expect("actual rival management can reinforce its racket before a challenge")
        .commit(&mut city.state)
        .unwrap();
    let strongest_force = u32::from(
        city.state
            .enterprises()
            .get_enterprise(racket_id)
            .unwrap()
            .associates(),
    );
    let force_by_district = BTreeMap::from([(rival_district, strongest_force)]);
    validate_suspend_enterprise(&city.state, city.starter_racket)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert!(
        predatory_stake(
            &registry,
            &city.state,
            rival,
            target,
            &force_by_district,
            strongest_force,
            &mut BTreeMap::new(),
        ) > 0,
        "suspending the last local racket must expose real, still-operating storefronts"
    );
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut city.state);
    assert_eq!(tick, next_day(&registry, &mut restored));
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
        "rival decisions must continue identically after save and load"
    );
    assert!(
        tick.autonomous_operations.iter().any(|id| {
            let operation = city.state.operations().get_operation(*id).unwrap();
            operation.kind() == OperationKind::Surveillance
                && operation
                    .objective()
                    .business_target()
                    .is_some_and(|business| {
                        city.state
                            .world()
                            .get_business(business)
                            .is_some_and(|record| {
                                record.owner() == BusinessOwner::Organization(player)
                            })
                    })
        }),
        "at least one rival must actually scout an exposed player-owned business"
    );
}

#[test]
fn paid_rival_crew_can_pressure_undefended_shops_without_active_rackets_or_competing_stores() {
    let Fixture {
        registry,
        mut state,
        player,
        ..
    } = fixture();
    let raiders = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Unestablished Raiders".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let captain = insert_character(
        &mut state,
        CharacterDraft {
            name: "Raider Captain".into(),
            organization: Some(raiders),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(85))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    insert_character(
        &mut state,
        CharacterDraft {
            name: "Raider Scout".into(),
            organization: Some(raiders),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, rating(90)),
                (CapabilityKind::Burglary, rating(85)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    validate_assign_mandate(
        &state,
        MandateDraft {
            organization: raiders,
            manager: captain,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Operations,
            )]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let mut understrength = state.clone();
    let first = next_day(&registry, &mut understrength);
    assert!(
        first.autonomous_operations.iter().all(|id| {
            understrength
                .operations()
                .get_operation(*id)
                .unwrap()
                .responsible_organization()
                != raiders
        }),
        "two named people cannot project enough criminal force"
    );
    let mut office_only = state.clone();
    for index in 0..4 {
        insert_character(
            &mut office_only,
            CharacterDraft {
                name: format!("Raider Administrator {index}"),
                organization: Some(raiders),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(CapabilityKind::Management, rating(70))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
    let office_tick = next_day(&registry, &mut office_only);
    assert!(
        office_tick.autonomous_operations.iter().all(|id| {
            office_only
                .operations()
                .get_operation(*id)
                .unwrap()
                .responsible_organization()
                != raiders
        }),
        "a payroll of administrators cannot create territorial fighting strength"
    );
    for index in 0..4 {
        insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Raider Soldier {index}"),
                organization: Some(raiders),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(CapabilityKind::Violence, rating(70))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
    let tick = next_day(&registry, &mut state);
    assert!(
        tick.autonomous_operations.iter().any(|id| {
            let operation = state.operations().get_operation(*id).unwrap();
            operation.responsible_organization() == raiders
                && operation.kind() == OperationKind::Surveillance
                && operation
                    .objective()
                    .business_target()
                    .is_some_and(|business| {
                        state.world().get_business(business).unwrap().owner()
                            == BusinessOwner::Organization(player)
                    })
        }),
        "a rival roster can target an undefended shop without owning a business or a racket"
    );
}

#[test]
fn rival_reassesses_market_after_a_concerted_campaign() {
    let Fixture {
        registry,
        mut state,
        rival,
        competitor,
        ..
    } = fixture_with_competitors(100, Some(BusinessKind::Pharmacy));
    let second = state
        .world()
        .public_business_directory()
        .find(|(_, name, _, _)| *name == "Second Finch Harbor Store")
        .unwrap()
        .0;
    for kind in [
        OperationKind::Surveillance,
        OperationKind::Sabotage,
        OperationKind::Sabotage,
    ] {
        let tick = next_day(&registry, &mut state);
        assert_eq!(tick.autonomous_operations.len(), 1);
        let id = tick.autonomous_operations[0];
        let record = state.operations().get_operation(id).unwrap();
        assert_eq!(record.responsible_organization(), rival);
        assert_eq!(record.kind(), kind);
        assert_eq!(record.objective().business_target(), Some(competitor));
        finish_operation(&registry, &mut state, id);
        assert!(
            state
                .operations()
                .get_operation(id)
                .unwrap()
                .resolution()
                .is_some_and(|outcome| outcome.objective_outcome()
                    != crate::operations::OperationObjectiveOutcome::Failed)
        );
    }
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut state);
    assert_eq!(tick, next_day(&registry, &mut restored));
    let record = state
        .operations()
        .get_operation(tick.autonomous_operations[0])
        .unwrap();
    assert_eq!(record.kind(), OperationKind::Surveillance);
    assert_eq!(record.objective().business_target(), Some(second));
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn rival_extends_profitable_strikes_against_a_prominent_public_owner() {
    let Fixture {
        registry,
        mut state,
        rival,
        player,
        competitor,
        ..
    } = fixture_with_competitors(100, Some(BusinessKind::Pharmacy));
    let elsewhere = state
        .world()
        .neighborhoods()
        .find(|ward| ward.name() == "Uptown")
        .unwrap()
        .id();
    for index in 0..8 {
        insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: format!("Public Expansion {index}"),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::ProfessionalRecords,
                ]),
                neighborhood: elsewhere,
                owner: BusinessOwner::Organization(player),
            },
        )
        .unwrap();
    }
    assert!(
        public_prominence_tier(&state, player) >= 3,
        "the public footprint must earn sustained strategic attention"
    );
    for kind in [
        OperationKind::Surveillance,
        OperationKind::Sabotage,
        OperationKind::Sabotage,
        OperationKind::Sabotage,
    ] {
        let tick = next_day(&registry, &mut state);
        assert_eq!(tick.autonomous_operations.len(), 1);
        let operation = tick.autonomous_operations[0];
        let record = state.operations().get_operation(operation).unwrap();
        assert_eq!(record.kind(), kind);
        assert_eq!(record.responsible_organization(), rival);
        assert_eq!(record.objective().business_target(), Some(competitor));
        finish_operation(&registry, &mut state, operation);
    }
    let restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn organized_rival_resists_public_political_expansion_without_shared_retail_sector() {
    let Fixture {
        registry,
        mut state,
        rival,
        player,
        competitor,
        ..
    } = fixture();
    let district = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Civic Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(60),
                    illicit_demand: rating(55),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(0),
                },
            },
        },
    )
    .unwrap();
    let mut operating_shop = |owner, name: &str, kind| {
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: name.to_owned(),
                kind,
                functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
                neighborhood: district,
                owner: BusinessOwner::Organization(owner),
            },
        )
        .unwrap();
        let operating_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .unwrap();
        let settlement_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_establish_business_economy(
            &registry,
            &state,
            BusinessEconomyDraft {
                business,
                operating_account,
                settlement_account,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        business
    };
    let rival_laundry = operating_shop(rival, "Rival Laundry", BusinessKind::Laundry);
    let civic_center = operating_shop(player, "Civic Retail Hall", BusinessKind::Retail);
    assert_eq!(
        forecast_competitor_disruption_gain(&registry, &state, civic_center, &[rival_laundry])
            .unwrap(),
        0,
        "rival has no direct retail customer incentive in the civic ward"
    );
    let mut quiet = state.clone();
    assert_eq!(
        next_day(&registry, &mut quiet)
            .autonomous_operations
            .iter()
            .map(|&id| quiet
                .operations()
                .get_operation(id)
                .unwrap()
                .objective()
                .business_target())
            .collect::<Vec<_>>(),
        vec![Some(competitor)],
        "without political stakes the rival should still pursue real customer gains"
    );

    insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Civic Council".to_owned(),
            kind: OrganizationKind::Political,
        },
    )
    .unwrap();
    let clean = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let source = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Civic test funding".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: clean,
                    amount: Money::from_cents(4_500_000),
                },
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-4_500_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    validate_invest_influence(&state, district, 4_500_000)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let mut absent = state.clone();
    crate::world::world_system::validate_transfer_business_ownership(
        &absent,
        rival_laundry,
        BusinessOwner::Independent,
    )
    .unwrap()
    .commit(&mut absent)
    .unwrap();
    let tick = next_day(&registry, &mut absent);
    assert_eq!(
        absent
            .world()
            .businesses_owned_by_organization(rival)
            .filter(|shop| shop.neighborhood() == district)
            .count(),
        0
    );
    assert_eq!(tick.autonomous_operations.len(), 1);
    assert_eq!(
        absent
            .operations()
            .get_operation(tick.autonomous_operations[0])
            .unwrap()
            .objective()
            .business_target(),
        Some(competitor),
        "political influence alone cannot give a rival a stake in an unrelated ward"
    );
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut state);
    assert_eq!(tick, next_day(&registry, &mut restored));
    assert_eq!(tick.autonomous_operations.len(), 1);
    let attack = state
        .operations()
        .get_operation(tick.autonomous_operations[0])
        .unwrap();
    assert_eq!(attack.kind(), OperationKind::Surveillance);
    assert_eq!(attack.responsible_organization(), rival);
    assert_eq!(attack.objective().business_target(), Some(civic_center));
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn held_business_settlement_stales_when_local_competitor_is_damaged() {
    use crate::economy::business_economy_system::{
        BusinessEconomyError, decide_business_cycle, validate_business_cycle_plan,
        validate_disrupt_business_economy,
    };
    let Fixture {
        registry,
        mut state,
        rival,
        competitor,
        ..
    } = fixture();
    let shop = state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .id();
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    let plan = decide_business_cycle(&registry, &state, shop, 0).unwrap();
    let token = validate_business_cycle_plan(&state, plan.clone()).unwrap();
    validate_disrupt_business_economy(&registry, &state, competitor)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let before = bincode::serialize(&state).unwrap();
    assert!(
        matches!(validate_business_cycle_plan(&state, plan), Err(BusinessEconomyError::StaleMarket(id)) if id == shop)
    );
    assert_eq!(
        token.commit(&mut state),
        Err(BusinessEconomyError::StaleMarket(shop))
    );
    assert_eq!(before, bincode::serialize(&state).unwrap());
    let plan = decide_business_cycle(&registry, &state, shop, 0).unwrap();
    let id = validate_business_cycle_plan(&state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    assert!(
        state
            .economy()
            .get_cycle(id)
            .unwrap()
            .market_gross_basis_points(&registry)
            == 8_857
    );
    restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
}

#[test]
fn rival_without_operational_authority_cannot_launch_pressure() {
    let Fixture {
        registry,
        mut state,
        manager,
        ..
    } = fixture();
    let mandate = state.delegation().active_for_manager(manager).unwrap().id();
    crate::delegation::delegation_system::validate_revoke_mandate(&state, mandate)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let tick = next_day(&registry, &mut state);
    assert!(tick.autonomous_operations.is_empty());
}

#[test]
fn rival_planner_rejects_untrained_scout() {
    let Fixture {
        registry,
        mut state,
        ..
    } = fixture_with_scout_rating(0);
    assert!(
        next_day(&registry, &mut state)
            .autonomous_operations
            .is_empty()
    );
}

#[test]
fn rival_planner_cooldown_starts_at_abort_not_authorization() {
    let Fixture {
        registry,
        mut state,
        ..
    } = fixture();
    let operation = next_day(&registry, &mut state).autonomous_operations[0];
    run_test_tick(&registry, &mut state);
    super::super::apply_transition(
        &registry,
        &mut state,
        operation,
        super::super::OperationTransition::Abort,
    )
    .unwrap();
    let retry_minutes = u32::try_from(UNSUCCESSFUL_RETRY_MINUTES - 1).unwrap();
    state.advance_clock(SimDuration::from_minutes(retry_minutes));
    assert!(apply_due_rival_operations(&registry, &mut state).is_empty());
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    assert_eq!(apply_due_rival_operations(&registry, &mut state).len(), 1);
}

#[test]
fn rival_stands_down_under_own_police_fear() {
    let Fixture {
        registry,
        mut state,
        rival,
        ..
    } = fixture();
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut state,
        rival,
        AudienceKind::Police,
        100,
    )
    .unwrap();
    let tick = next_day(&registry, &mut state);
    assert!(tick.autonomous_operations.is_empty());
}
