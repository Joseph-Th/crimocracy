//! Enterprise host and supporting-business network requirements and ownership lifecycle.

use super::*;

#[test]
fn bookmaking_requires_a_racing_wire_beyond_the_betting_front() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let betting_front = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Horse Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::RacingWire,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Bookmaking,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(betting_front),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("the betting front must not substitute for a separate racing-wire service"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::RacingWire,
        }
    );

    let wire_service = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Racing News",
        BusinessKind::NewsService,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::RacingWire,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Bookmaking,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(betting_front),
            supporting_businesses: BTreeSet::from([wire_service]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("betting front plus racing wire should support bookmaking")
    .commit(&mut fixture.state)
    .expect("validated bookmaking enterprise should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("bookmaking enterprise should persist");
    assert_eq!(record.kind(), EnterpriseKind::Bookmaking);
    assert_eq!(
        record.supporting_businesses(),
        &BTreeSet::from([wire_service])
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("bookmaking wire network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn fraud_requires_a_financial_records_front_and_customer_channel() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let generic_office = insert_support_business(
        &registry,
        &mut fixture,
        "Ordinary Accountancy",
        BusinessKind::ProfessionalServices,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Fraud,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(generic_office),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("ordinary records must not stand in for financial-paper access"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: generic_office,
            function: BusinessFunction::FinancialServices,
        }
    );

    let finance_office = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Finance & Indemnity",
        BusinessKind::FinancialServices,
        BTreeSet::from([
            BusinessFunction::FinancialServices,
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Fraud,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(finance_office),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("a financial front may supply its own customer channel for commercial fraud")
    .commit(&mut fixture.state)
    .expect("validated fraud enterprise should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("fraud enterprise should persist");
    assert_eq!(record.kind(), EnterpriseKind::Fraud);
    assert_eq!(
        record.location(),
        EnterpriseLocation::Business(finance_office)
    );
    assert!(record.supporting_businesses().is_empty());
    validate_state_against_registry(&registry, &fixture.state)
        .expect("fraud network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn auto_theft_ring_requires_a_workshop_and_vehicle_disposal_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let generic_dealer = insert_support_business(
        &registry,
        &mut fixture,
        "Ordinary Used Car Lot",
        BusinessKind::Automotive,
        BTreeSet::from([
            BusinessFunction::CustomerAccess,
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::ResaleMarket,
            BusinessFunction::VehicleFleet,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AutoTheftRing,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(generic_dealer),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => {
            panic!("a dealership without workshop capability must not re-identify stolen cars")
        }
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: generic_dealer,
            function: BusinessFunction::VehicleWorkshop,
        }
    );

    let workshop = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Motor Works",
        BusinessKind::Automotive,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::ResaleMarket,
            BusinessFunction::VehicleFleet,
            BusinessFunction::VehicleWorkshop,
        ]),
        BusinessOwner::Organization(organization),
    );
    let sales_front = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Auto Sales",
        BusinessKind::Automotive,
        BTreeSet::from([
            BusinessFunction::CustomerAccess,
            BusinessFunction::ResaleMarket,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AutoTheftRing,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(workshop),
            supporting_businesses: BTreeSet::from([sales_front]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("workshop, records, and customer access should support a stolen-auto ring")
    .commit(&mut fixture.state)
    .expect("validated stolen-auto ring should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("stolen-auto ring should persist");
    assert_eq!(record.kind(), EnterpriseKind::AutoTheftRing);
    assert_eq!(record.location(), EnterpriseLocation::Business(workshop));
    assert_eq!(
        record.supporting_businesses(),
        &BTreeSet::from([sales_front])
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("stolen-auto network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn smuggling_requires_real_dock_access_and_a_documented_distribution_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let generic_warehouse = insert_support_business(
        &registry,
        &mut fixture,
        "Inland Freight Warehouse",
        BusinessKind::Warehouse,
        BTreeSet::from([
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Smuggling,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(generic_warehouse),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("an inland warehouse must not stand in for waterfront access"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: generic_warehouse,
            function: BusinessFunction::DockAccess,
        }
    );

    let stevedore = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Pier & Stevedoring",
        BusinessKind::Stevedoring,
        BTreeSet::from([
            BusinessFunction::DockAccess,
            BusinessFunction::Warehousing,
            BusinessFunction::UnionAccess,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let carrier = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Cartage Company",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::ProfessionalRecords,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Smuggling,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(stevedore),
            supporting_businesses: BTreeSet::from([carrier]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("dock storage plus documented carrier access should support smuggling")
    .commit(&mut fixture.state)
    .expect("validated smuggling enterprise should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("smuggling enterprise should persist");
    assert_eq!(record.kind(), EnterpriseKind::Smuggling);
    assert_eq!(record.location(), EnterpriseLocation::Business(stevedore));
    assert_eq!(record.supporting_businesses(), &BTreeSet::from([carrier]));
    validate_state_against_registry(&registry, &fixture.state)
        .expect("smuggling network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn counterfeiting_requires_a_real_press_and_a_separate_passing_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let generic_office = insert_support_business(
        &registry,
        &mut fixture,
        "Generic Records Office",
        BusinessKind::ProfessionalServices,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Counterfeiting,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(generic_office),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("records alone must not stand in for a printing press"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: generic_office,
            function: BusinessFunction::PrintingPress,
        }
    );

    let printer = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Commercial Printing",
        BusinessKind::Printing,
        BTreeSet::from([
            BusinessFunction::PrintingPress,
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Counterfeiting,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(printer),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("the print shop must not substitute for a separate passing network"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::CustomerAccess,
        }
    );
    let distributor = insert_support_business(
        &registry,
        &mut fixture,
        "Ward General Wholesale",
        BusinessKind::Wholesale,
        BTreeSet::from([
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::Warehousing,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Counterfeiting,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(printer),
            supporting_businesses: BTreeSet::from([distributor]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("printing front plus commercial distribution should support counterfeiting")
    .commit(&mut fixture.state)
    .expect("validated counterfeiting enterprise should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("counterfeiting enterprise should persist");
    assert_eq!(record.kind(), EnterpriseKind::Counterfeiting);
    assert_eq!(record.location(), EnterpriseLocation::Business(printer));
    assert_eq!(
        record.supporting_businesses(),
        &BTreeSet::from([distributor])
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("counterfeiting network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn brothel_requires_a_lodging_front_instead_of_any_cash_venue() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let generic_venue = insert_support_business(
        &registry,
        &mut fixture,
        "Generic Cash Club",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Brothel,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(generic_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("a generic cash venue without lodging must not host a brothel"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: generic_venue,
            function: BusinessFunction::Lodging,
        }
    );

    let lodging = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Boarding Hotel",
        BusinessKind::Lodging,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::Lodging,
            BusinessFunction::MeetingSpace,
        ]),
        BusinessOwner::Organization(organization),
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_business(lodging)
            .expect("lodging front should persist")
            .neighborhood(),
        neighborhood
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Brothel,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(lodging),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("a cash-handling lodging front should host a brothel")
    .commit(&mut fixture.state)
    .expect("validated brothel should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("brothel should persist")
            .kind(),
        EnterpriseKind::Brothel
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("brothel front should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn prizefighting_uses_an_athletic_host_and_separate_cash_front_when_needed() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let athletic_club = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Athletic Club",
        BusinessKind::AthleticClub,
        BTreeSet::from([
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
            BusinessFunction::SportingVenue,
        ]),
        BusinessOwner::Organization(organization),
    );
    let incomplete = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::PrizeFighting,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(athletic_club),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("a fight venue without cash handling must have a support front"),
        Err(error) => error,
    };
    assert_eq!(
        incomplete,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::CashIntensive,
        }
    );

    let laundry = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Laundry Service",
        BusinessKind::Laundry,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::VehicleFleet,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::PrizeFighting,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(athletic_club),
            supporting_businesses: BTreeSet::from([laundry]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("athletic venue plus cash front should support prizefighting")
    .commit(&mut fixture.state)
    .expect("validated prizefighting enterprise should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("prizefighting enterprise should persist");
    assert_eq!(record.kind(), EnterpriseKind::PrizeFighting);
    assert_eq!(
        record.location(),
        EnterpriseLocation::Business(athletic_club)
    );
    assert_eq!(record.supporting_businesses(), &BTreeSet::from([laundry]));
    validate_state_against_registry(&registry, &fixture.state)
        .expect("prizefighting network should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn slot_machine_route_uses_distribution_network_without_route_micromanagement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let distributor = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Coin Machine Company",
        BusinessKind::CoinMachineDistribution,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let customer_front = insert_support_business(
        &registry,
        &mut fixture,
        "Corner Pawn & Loan",
        BusinessKind::Pawnshop,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::ResaleMarket,
        ]),
        BusinessOwner::Organization(organization),
    );

    let incomplete = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::SlotMachineRoute,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([distributor]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("a distributor without cash handling must not support a slot route"),
        Err(error) => error,
    };
    assert_eq!(
        incomplete,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::CashIntensive,
        }
    );

    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::SlotMachineRoute,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([distributor, customer_front]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("distribution and customer access should support a district slot-machine route")
    .commit(&mut fixture.state)
    .expect("validated slot-machine route should commit");

    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("slot-machine route should persist");
    assert_eq!(record.kind(), EnterpriseKind::SlotMachineRoute);
    assert_eq!(record.location(), fixture.location);
    assert_eq!(
        record.supporting_businesses(),
        &BTreeSet::from([distributor, customer_front])
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("slot-machine route network should satisfy authored content");
    validate_invariants(&fixture.state);
}

#[test]
fn numbers_racket_uses_a_front_network_without_a_dedicated_gambling_venue() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let collection_front = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Wholesale Counter",
        BusinessKind::Wholesale,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let policy_office = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Policy Office",
        BusinessKind::ProfessionalServices,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::MeetingSpace,
        ]),
        BusinessOwner::Organization(organization),
    );

    let incomplete = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::NumbersRacket,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([collection_front]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("a collection front without the policy bank's records must be incomplete"),
        Err(error) => error,
    };
    assert_eq!(
        incomplete,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::ProfessionalRecords,
        }
    );

    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::NumbersRacket,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([collection_front, policy_office]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("cash-handling customer network should support a neighborhood numbers racket")
    .commit(&mut fixture.state)
    .expect("validated numbers racket should commit");

    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("numbers racket should persist");
    assert_eq!(record.kind(), EnterpriseKind::NumbersRacket);
    assert_eq!(record.location(), fixture.location);
    assert_eq!(
        record.supporting_businesses(),
        &BTreeSet::from([collection_front, policy_office])
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("numbers-racket network should satisfy authored content");
    validate_invariants(&fixture.state);
}

#[test]
fn repeal_pivot_rackets_establish_through_distinct_production_networks() {
    // Research-backed content: Empire of Sin shipped without drug, elite-extortion, or
    // city-hall corruption systems, while Gangsters: Organized Crime proved charity, union,
    // and mayoral paths carry a campaign. These three rackets give the organization a
    // post-Prohibition future with non-overlapping infrastructure instead of another
    // interchangeable income node.
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let owner = BusinessOwner::Organization(organization);

    // Narcotics is a hosted racket: the pharmacy front holds the cash trade while a
    // distribution back-office moves supply through the wider network.
    let pharmacy = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Pharmacy",
        BusinessKind::Pharmacy,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::ProfessionalRecords,
        ]),
        owner,
    );
    let depot = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Depot",
        BusinessKind::Wholesale,
        BTreeSet::from([BusinessFunction::DistributionInfrastructure]),
        owner,
    );
    let narcotics_location = EnterpriseLocation::Business(pharmacy);
    let narcotics = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::NarcoticsTrade,
            organization,
            authority: fixture.authority,
            location: narcotics_location,
            supporting_businesses: BTreeSet::from([depot]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("pharmacy and distribution network should support a narcotics trade")
    .commit(&mut fixture.state)
    .expect("validated narcotics trade should commit");

    // Blackmail is a hosted racket: a discreet records office runs the extortion while a
    // crowd venue supplies victims and lodging supplies compromising premises.
    let blackmail_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("blackmail settlement account should validate");
    let office = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Records Office",
        BusinessKind::ProfessionalServices,
        BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::MeetingSpace,
        ]),
        owner,
    );
    let theater = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Picture Palace",
        BusinessKind::MovieTheater,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::MeetingSpace,
        ]),
        owner,
    );
    let boarding = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Boarding House",
        BusinessKind::Lodging,
        BTreeSet::from([BusinessFunction::Lodging, BusinessFunction::MeetingSpace]),
        owner,
    );
    let blackmail_location = EnterpriseLocation::Business(office);
    let blackmail = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Blackmail,
            organization,
            authority: fixture.authority,
            location: blackmail_location,
            supporting_businesses: BTreeSet::from([theater, boarding]),
            cash_account: fixture.cash,
            settlement_account: blackmail_settlement,
        },
    )
    .expect("records office, crowd venue, and lodging should support blackmail")
    .commit(&mut fixture.state)
    .expect("validated blackmail should commit");

    // Municipal graft pairs union access with a financial-services office.
    let graft_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("graft settlement account should validate");
    let contractor = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Contractor",
        BusinessKind::Construction,
        BTreeSet::from([BusinessFunction::UnionAccess]),
        owner,
    );
    let office = insert_support_business(
        &registry,
        &mut fixture,
        "Ward Finance Office",
        BusinessKind::FinancialServices,
        BTreeSet::from([
            BusinessFunction::FinancialServices,
            BusinessFunction::ProfessionalRecords,
        ]),
        owner,
    );
    let graft = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::MunicipalGraft,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([contractor, office]),
            cash_account: fixture.cash,
            settlement_account: graft_settlement,
        },
    )
    .expect("union and financial network should support municipal graft")
    .commit(&mut fixture.state)
    .expect("validated municipal graft should commit");

    for (id, kind, location) in [
        (
            narcotics,
            EnterpriseKind::NarcoticsTrade,
            narcotics_location,
        ),
        (blackmail, EnterpriseKind::Blackmail, blackmail_location),
        (graft, EnterpriseKind::MunicipalGraft, fixture.location),
    ] {
        let record = fixture
            .state
            .enterprises()
            .get_enterprise(id)
            .expect("new racket should persist");
        assert_eq!(record.kind(), kind);
        assert_eq!(record.location(), location);
    }
    validate_state_against_registry(&registry, &fixture.state)
        .expect("new racket networks should satisfy authored content");
    validate_invariants(&fixture.state);
}

#[test]
fn business_hosted_gambling_requires_concrete_venue_functions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let incomplete_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Sparse Storefront".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("incomplete venue should still be a valid business");

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(incomplete_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("gambling must reject a venue without its required functions"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: incomplete_venue,
            function: BusinessFunction::CashIntensive,
        }
    );

    let valid_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Market Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("complete venue should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(valid_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("gambling should accept a venue with all required functions")
    .commit(&mut fixture.state)
    .expect("business-hosted enterprise should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should exist")
            .location(),
        EnterpriseLocation::Business(valid_venue)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn alcohol_distribution_uses_owned_business_network_and_survives_save_before_cycle() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let enterprise = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail]),
    )
    .expect("complete owned distribution network should establish");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_supported_by_business(transport)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![enterprise]
    );
    validate_state(&fixture.state).expect("alcohol distribution state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("alcohol distribution network should satisfy authored content");
    validate_invariants(&fixture.state);

    let save = build_save(&registry, &fixture.state)
        .expect("alcohol distribution state should build a save");
    let bytes = bincode::serialize(&save).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored = restore_save(&registry, decoded)
        .expect("alcohol distribution support indexes should restore");
    assert_eq!(
        restored
            .enterprises()
            .enterprises_supported_by_business(retail)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![enterprise]
    );

    restored.advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &restored,
        enterprise,
        EnterpriseCycleRandomness::new(
            0,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .expect("valid alcohol distribution network should resolve a due cycle");
    assert_eq!(
        plan.economics.net_cash,
        plan.economics
            .gross_revenue
            .checked_sub(plan.economics.operating_cost)
            .expect("net should be gross - cost")
    );
    assert!(plan.economics.gross_revenue.cents() > plan.economics.operating_cost.cents());
    validate_enterprise_cycle_plan(&restored, plan)
        .expect("fresh alcohol distribution cycle should validate")
        .commit(&mut restored)
        .expect("alcohol distribution cycle should commit");
    validate_state(&restored).expect("resolved alcohol distribution state should validate");
    validate_state_against_registry(&registry, &restored)
        .expect("resolved alcohol distribution state should remain authored-valid");
    validate_invariants(&restored);
}

#[test]
fn alcohol_distribution_rejects_incomplete_or_foreign_support_networks() {
    let registry = build_registry();
    let mut incomplete = make_test_enterprise_fixture();
    let incomplete_organization = incomplete.organization;
    let transport = insert_support_business(
        &registry,
        &mut incomplete,
        "Incomplete Freight Network",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(incomplete_organization),
    );
    let error =
        establish_alcohol_distribution(&registry, &mut incomplete, BTreeSet::from([transport]))
            .expect_err("distribution network without retail access must be rejected");
    assert_eq!(
        error,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::CustomerAccess,
        }
    );

    let mut foreign = make_test_enterprise_fixture();
    let network = insert_support_business(
        &registry,
        &mut foreign,
        "Independent Distribution Combine",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Independent,
    );
    let error = establish_alcohol_distribution(&registry, &mut foreign, BTreeSet::from([network]))
        .expect_err("foreign business capacity must not be consumed implicitly");
    assert_eq!(
        error,
        EnterpriseError::SupportingBusinessOwnershipMismatch {
            business: network,
            owner: BusinessOwner::Independent,
            organization: foreign.organization,
        }
    );
    validate_state(&incomplete.state).expect("rejected incomplete network should not mutate");
    validate_state(&foreign.state).expect("rejected foreign network should not mutate");
    validate_invariants(&incomplete.state);
    validate_invariants(&foreign.state);
}

#[test]
fn establishment_rejects_support_that_serves_no_authored_network_requirement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let unnecessary = insert_support_business(
        &registry,
        &mut fixture,
        "Unneeded Back Office",
        BusinessKind::ProfessionalServices,
        BTreeSet::from([BusinessFunction::ProfessionalRecords]),
        BusinessOwner::Organization(organization),
    );

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([unnecessary]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("a racket with no network requirement must not lock or surcharge support"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::RedundantSupportingBusiness {
            business: unnecessary,
        }
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);
    validate_invariants(&fixture.state);
}

#[test]
fn establishment_rejects_redundant_support_inside_a_complete_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let organization = fixture.organization;
    let unnecessary = insert_support_business(
        &registry,
        &mut fixture,
        "Unneeded Meeting Hall",
        BusinessKind::Hospitality,
        BTreeSet::from([BusinessFunction::MeetingSpace]),
        BusinessOwner::Organization(organization),
    );

    let error = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail, unnecessary]),
    )
    .expect_err(
        "complete networks must reject surcharge-producing support that contributes nothing",
    );
    assert_eq!(
        error,
        EnterpriseError::RedundantSupportingBusiness {
            business: unnecessary,
        }
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_persisted_enterprise_with_redundant_support_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let enterprise = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail]),
    )
    .expect("complete distribution network should establish");
    let original = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("distribution enterprise should persist")
        .clone();
    let mut corrupted = enterprise_wire(&original);
    // Protection has no authored support-network requirement. Rewriting only the fixed-width kind
    // discriminator preserves a decodable save while making both persisted support links
    // semantically redundant, a state canonical establishment can never create.
    corrupted.identity.kind = EnterpriseKind::Protection;
    let envelope = replace_serialized_enterprise(
        build_save(&registry, &fixture.state).expect("valid distribution network should save"),
        &original,
        &corrupted,
    );
    let error = restore_save(&registry, envelope)
        .expect_err("registry-relative restore validation must reject redundant persisted support");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::RedundantEnterpriseSupportingBusiness {
                enterprise: invalid,
                business,
            }
        ) if invalid == enterprise && business == transport
    ));
}

#[test]
fn neighborhood_authority_cannot_bind_support_business_in_another_district() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let local_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood authority"),
    };
    assert_eq!(
        fixture.authority.scope,
        ResponsibilityScope::Neighborhood(local_neighborhood)
    );
    let remote_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Remote Logistics Ward".to_owned(),
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
    .expect("remote neighborhood should validate");
    let organization = fixture.organization;
    let remote_transport = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Remote Freight Depot".to_owned(),
            kind: BusinessKind::Transportation,
            functions: BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
            ]),
            neighborhood: remote_neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("remote transport should validate");
    let local_retail = insert_support_business(
        &registry,
        &mut fixture,
        "Local Bottle Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([remote_transport, local_retail]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("district authority must not bind remote support infrastructure"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::AuthoritySupportingBusinessMismatch {
            scope: fixture.authority.scope,
            business: remote_transport,
        }
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);
    validate_state(&fixture.state).expect("rejected remote support must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn distribution_establishment_token_stales_when_support_ownership_changes() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let expected_version = fixture
        .state
        .world()
        .get_business(retail)
        .expect("support business should exist")
        .version();
    let establishment = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([transport, retail]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("complete distribution network should initially validate");

    validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
        .expect("no committed enterprise should lock support ownership yet")
        .commit(&mut fixture.state)
        .expect("support ownership transfer should commit before enterprise establishment");
    let found_version = fixture
        .state
        .world()
        .get_business(retail)
        .expect("support business should remain")
        .version();
    assert_eq!(
        establishment
            .commit(&mut fixture.state)
            .expect_err("support mutation must stale validated establishment"),
        EnterpriseError::StaleSupportingBusiness {
            business: retail,
            expected: expected_version,
            found: found_version,
        }
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_supported_by_business(transport)
            .count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .expect("settlement account should persist")
            .balance(),
        Money::ZERO
    );
    validate_state(&fixture.state)
        .expect("stale establishment rejection should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn active_distribution_network_locks_business_ownership_and_resume_revalidates_versions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let enterprise = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail]),
    )
    .expect("complete network should establish");

    let error =
        validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
            .expect_err("active enterprise must lock supporting business ownership");
    assert_eq!(
        error,
        WorldError::ActiveEnterpriseSupport {
            business: retail,
            enterprise,
            organization: fixture.organization,
        }
    );

    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active distribution enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("distribution suspension should commit");
    let stale_resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("owned support network should initially validate for resume");
    validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
        .expect("suspended enterprise should release support ownership lock")
        .commit(&mut fixture.state)
        .expect("support ownership transfer should commit while suspended");
    assert_eq!(
        stale_resume
            .commit(&mut fixture.state)
            .expect_err("support ownership mutation must stale prior resume token"),
        EnterpriseError::StaleSupportingBusiness {
            business: retail,
            expected: 1,
            found: 2,
        }
    );
    let fresh_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("foreign-owned support network must not resume"),
        Err(error) => error,
    };
    assert_eq!(
        fresh_error,
        EnterpriseError::SupportingBusinessOwnershipMismatch {
            business: retail,
            owner: BusinessOwner::Independent,
            organization: fixture.organization,
        }
    );

    validate_transfer_business_ownership(
        &fixture.state,
        retail,
        BusinessOwner::Organization(fixture.organization),
    )
    .expect("suspended support business should be transferable back")
    .commit(&mut fixture.state)
    .expect("support ownership restoration should commit");
    validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("restored network should resume")
        .commit(&mut fixture.state)
        .expect("restored distribution enterprise resume should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("distribution enterprise should persist")
            .status(),
        EnterpriseStatus::Active
    );
    validate_state(&fixture.state).expect("restored distribution network should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("restored distribution network should satisfy authored content");
    validate_invariants(&fixture.state);
}

#[test]
fn resume_commit_rejects_a_host_venue_that_changed_after_validation() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Resumption Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("host venue fixture should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("business-hosted gambling should establish")
    .commit(&mut fixture.state)
    .expect("business-hosted gambling should commit");
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active venue racket should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    // A resume token validated while the organization still owned the venue must not
    // reactivate the racket after the venue changed hands: the host version pin stales it.
    let stale_resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("owned venue should initially validate for resume");
    validate_transfer_business_ownership(&fixture.state, venue, BusinessOwner::Independent)
        .expect("suspended enterprise should release the host lock")
        .commit(&mut fixture.state)
        .expect("venue sale should commit while suspended");
    assert_eq!(
        stale_resume
            .commit(&mut fixture.state)
            .expect_err("a sold venue must stale the prior resume token"),
        EnterpriseError::StaleHostBusiness {
            business: venue,
            expected: 1,
            found: 2,
        }
    );
    let fresh_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("a foreign-owned venue must not host a resumed racket"),
        Err(error) => error,
    };
    assert_eq!(
        fresh_error,
        EnterpriseError::HostBusinessOwnershipMismatch {
            business: venue,
            owner: BusinessOwner::Independent,
            organization: fixture.organization,
        }
    );
    validate_state(&fixture.state).expect("rejected resume should leave valid state");
    validate_invariants(&fixture.state);
}
