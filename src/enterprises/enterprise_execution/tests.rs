//! Focused tests for `enterprise_execution` lifecycle, settlement, and reporting.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::simulation::run_tick;
use crate::delegation::delegation_system::{
    validate_assign_mandate, validate_revise_mandate, validate_revoke_mandate, DelegationError,
    MandateRevisionDraft,
};
use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
use crate::enterprises::enterprise_reporting::{
    resolve_enterprise_financial_summary, resolve_manager_enterprise_financial_summary,
    resolve_neighborhood_enterprise_financial_summary,
    resolve_organization_enterprise_financial_summary,
};
use crate::enterprises::EnterpriseKind;
use crate::finance::finance_system::insert_account;
use crate::finance::{FinancialAccountDraft, FinancialOwner};
use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
use crate::legal::investigation_system::{
    validate_add_evidence, validate_incident_intake, validate_open_investigation,
};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft, JurisdictionDraft,
};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
};
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
    validate_transfer_business_ownership, WorldError,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    OrganizationDraft, OrganizationKind,
};
use std::collections::{BTreeMap, BTreeSet};

struct EnterpriseFixture {
    state: AppState,
    authority: MandateAuthority,
    organization: OrganizationId,
    location: EnterpriseLocation,
    cash: FinancialAccountId,
    settlement: FinancialAccountId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_test_enterprise_fixture() -> EnterpriseFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xE17E_1931);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Enterprise Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Market Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Enterprise Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager fixture should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate fixture should validate")
    .commit(&mut state)
    .expect("mandate fixture should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("cash account fixture should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account fixture should validate");
    EnterpriseFixture {
        state,
        authority: MandateAuthority {
            mandate,
            manager,
            scope: ResponsibilityScope::Neighborhood(neighborhood),
        },
        organization,
        location: EnterpriseLocation::Neighborhood(neighborhood),
        cash,
        settlement,
    }
}

fn establish_protection(registry: &Registry, fixture: &mut EnterpriseFixture) -> EnterpriseId {
    validate_establish_enterprise(
        registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("enterprise fixture should validate")
    .commit(&mut fixture.state)
    .expect("enterprise fixture should commit")
}

fn insert_support_business(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    name: &str,
    kind: BusinessKind,
    functions: BTreeSet<BusinessFunction>,
    owner: BusinessOwner,
) -> BusinessId {
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    insert_business(
        registry,
        &mut fixture.state,
        BusinessDraft {
            name: name.to_owned(),
            kind,
            functions,
            neighborhood,
            owner,
        },
    )
    .expect("support business fixture should validate")
}

fn alcohol_support_network(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
) -> (BusinessId, BusinessId) {
    let transport = insert_support_business(
        registry,
        fixture,
        "Harbor Freight & Storage",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(fixture.organization),
    );
    let retail = insert_support_business(
        registry,
        fixture,
        "Neighborhood Bottle Shop",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(fixture.organization),
    );
    (transport, retail)
}

fn establish_alcohol_distribution(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    supporting_businesses: BTreeSet<BusinessId>,
) -> Result<EnterpriseId, EnterpriseError> {
    validate_establish_enterprise(
        registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses,
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )?
    .commit(&mut fixture.state)
}

#[test]
fn routine_cycle_records_causal_economics_and_balanced_cash_settlement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, 0)
        .expect("due enterprise cycle should resolve");
    assert_eq!(
        plan.net_cash(),
        plan.gross_revenue()
            .checked_sub(plan.operating_cost())
            .expect("net should be gross - cost")
    );
    assert!(plan.gross_revenue().cents() > 0);
    assert!(plan.operating_cost().cents() > 0);

    let cycle = validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("cycle settlement should commit");
    let cycle_record = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("cycle should exist");
    assert!(cycle_record.transaction().is_some());
    let cash_balance = fixture
        .state
        .finance()
        .get_account(fixture.cash)
        .expect("cash account should exist")
        .balance();
    let settlement_balance = fixture
        .state
        .finance()
        .get_account(fixture.settlement)
        .expect("settlement account should exist")
        .balance();
    assert_eq!(cash_balance, cycle_record.net_cash());
    assert_eq!(settlement_balance, Money::from_cents(-cash_balance.cents()));
    validate_invariants(&fixture.state);
}

#[test]
fn district_heat_surcharge_scopes_to_the_enterprise_neighborhood() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let local_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(neighborhood) => neighborhood,
        EnterpriseLocation::Business(_) => {
            panic!("enterprise fixture should be located in a neighborhood")
        }
    };
    let other_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Dock Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("other neighborhood fixture should validate");
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Metro Police Authority".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([local_neighborhood, other_neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .expect("spanning jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("spanning jurisdiction should commit");
    let manager = fixture.authority.manager;

    let open_heat_case = |fixture: &mut EnterpriseFixture, title: &str, target| {
        let origin = validate_authorize_operation(
            &registry,
            &fixture.state,
            OperationDraft {
                title: format!("{title} origin patrol"),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.organization,
                leader: manager,
                objective: OperationObjective::GatherInformation { target },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, manager)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("origin operation should validate")
        .commit(&mut fixture.state)
        .expect("origin operation should commit");
        validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: police,
                title: title.to_owned(),
                subjects: BTreeSet::from([
                    EntityRef::Operation(origin),
                    EntityRef::Character(manager),
                ]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Character(manager),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin_operation: Some(origin),
                notified_organizations: BTreeSet::from([fixture.organization]),
                witness: None,
            },
        )
        .expect("incident intake should validate")
        .commit(&mut fixture.state)
        .expect("incident intake should commit");
    };
    let due_cycle = |fixture: &mut EnterpriseFixture| {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, 0)
            .expect("due enterprise cycle should resolve");
        let (cost, heat, attention) = (
            plan.operating_cost(),
            plan.investigation_heat(),
            plan.attention(),
        );
        validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("cycle plan should validate")
            .commit(&mut fixture.state)
            .expect("cycle settlement should commit");
        (cost, heat, attention)
    };

    let (baseline_cost, baseline_heat, baseline_attention) = due_cycle(&mut fixture);
    assert_eq!(baseline_heat, Money::ZERO);
    assert_eq!(baseline_attention, AttentionClass::Routine);
    // A case targeting another district of the same authority must not tax this racket.
    open_heat_case(
        &mut fixture,
        "Dock ward inquiry",
        EntityRef::Neighborhood(other_neighborhood),
    );
    let (cross_district_cost, cross_district_heat, _) = due_cycle(&mut fixture);
    assert_eq!(cross_district_cost, baseline_cost);
    assert_eq!(cross_district_heat, Money::ZERO);
    // A case targeting the enterprise's own district raises the daily cost by $50, becomes
    // notable, and records a player-visible report explaining the street surcharge.
    open_heat_case(
        &mut fixture,
        "Market ward inquiry",
        EntityRef::Neighborhood(local_neighborhood),
    );
    let (local_cost, local_heat, local_attention) = due_cycle(&mut fixture);
    assert_eq!(
        local_cost,
        cross_district_cost
            .checked_add(Money::from_cents(5_000))
            .expect("heat surcharge arithmetic should not overflow")
    );
    assert_eq!(local_heat, Money::from_cents(5_000));
    assert_eq!(local_attention, AttentionClass::Notable);
    let hot_cycle = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .max_by_key(|cycle| cycle.occurred_at())
        .expect("hot cycle should persist");
    let hot_information = fixture
        .state
        .intelligence()
        .get_information(
            hot_cycle
                .information()
                .expect("hot cycle must carry its report"),
        )
        .expect("cycle report information must persist");
    assert!(hot_information
        .summary()
        .contains("$50.00 street surcharge while police work stays heavy"));
    validate_state(&fixture.state).expect("district heat state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn detained_enterprise_manager_pauses_due_cycles_until_release() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let manager = fixture.authority.manager;
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Enterprise Custody Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: police,
            title: "Enterprise manager custody inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(manager)]),
        },
    )
    .expect("custody investigation should validate")
    .commit(&mut fixture.state)
    .expect("custody investigation should commit");
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(manager),
            origin: None,
            kind: EvidenceKind::FinancialRecord,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("custody evidence should validate")
    .commit(&mut fixture.state)
    .expect("custody evidence should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let arrest = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: manager,
            investigation,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("manager arrest should not require revoking formal enterprise authority")
    .commit(&mut fixture.state)
    .expect("manager arrest should commit");

    assert!(find_due_enterprises(&fixture.state).is_empty());
    let detained_tick = run_tick(&registry, &mut fixture.state);
    assert!(detained_tick.enterprise_cycles.is_empty());
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should persist")
            .next_cycle_at(),
        Some(SimTime::from_minutes(1_440))
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(2_880));
    let still_detained_tick = run_tick(&registry, &mut fixture.state);
    assert!(still_detained_tick.enterprise_cycles.is_empty());
    validate_state(&fixture.state).expect("paused enterprise detention state should validate");
    validate_invariants(&fixture.state);

    validate_release_arrest(&fixture.state, arrest)
        .expect("manager detention should release")
        .commit(&mut fixture.state)
        .expect("manager release should commit");
    let released_tick = run_tick(&registry, &mut fixture.state);
    assert_eq!(released_tick.enterprise_cycles.len(), 1);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_cycle(released_tick.enterprise_cycles[0])
            .expect("released manager should produce the overdue enterprise cycle")
            .enterprise(),
        enterprise
    );
    let next_cycle_at = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should persist after release")
        .next_cycle_at();
    assert_eq!(
        next_cycle_at,
        Some(fixture.state.now() + SimDuration::from_minutes(1_440))
    );
    let no_burst_tick = run_tick(&registry, &mut fixture.state);
    assert!(no_burst_tick.enterprise_cycles.is_empty());
    validate_state(&fixture.state).expect("resumed enterprise state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn settlement_account_is_exclusive_to_one_enterprise_history() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let first = establish_protection(&registry, &mut fixture);

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("settlement account reuse must fail before mutation"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::SettlementAccountInUse {
            account: fixture.settlement,
            enterprise: first,
        }
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_at(fixture.location)
            .count(),
        1
    );
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
    let plan = decide_enterprise_cycle(&registry, &restored, enterprise, 0)
        .expect("valid alcohol distribution network should resolve a due cycle");
    assert_eq!(
        plan.net_cash(),
        plan.gross_revenue()
            .checked_sub(plan.operating_cost())
            .expect("net should be gross - cost")
    );
    assert!(plan.gross_revenue().cents() > plan.operating_cost().cents());
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
fn suspension_removes_enterprise_from_due_work_and_resume_reschedules_it() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(find_due_enterprises(&fixture.state).is_empty());

    let resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("suspended enterprise with valid authority should resume");
    fixture.state.advance_clock(SimDuration::from_minutes(30));
    resume
        .commit(&mut fixture.state)
        .expect("resume should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should exist");
    assert_eq!(record.status(), EnterpriseStatus::Active);
    assert_eq!(
        record.next_cycle_at(),
        Some(fixture.state.now() + SimDuration::from_minutes(1_440))
    );
    validate_invariants(&fixture.state);
}

#[test]
fn enterprise_establishment_schedule_starts_at_commit_time() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let establishment = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("enterprise should validate before delayed commit");
    fixture.state.advance_clock(SimDuration::from_minutes(60));
    let enterprise = establishment
        .commit(&mut fixture.state)
        .expect("delayed enterprise establishment should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should exist");
    assert_eq!(record.established_at(), SimTime::from_minutes(60));
    assert_eq!(record.next_cycle_at(), Some(SimTime::from_minutes(1_500)));
    validate_invariants(&fixture.state);
}

#[test]
fn stale_cycle_plan_cannot_commit_after_enterprise_lifecycle_change() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, 0)
        .expect("cycle should resolve");
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    let error = match validate_enterprise_cycle_plan(&fixture.state, plan) {
        Ok(_) => panic!("cycle plan must become stale after lifecycle mutation"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::StaleEnterprise {
            enterprise,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should exist")
            .balance(),
        Money::ZERO
    );
    validate_invariants(&fixture.state);
}

#[test]
fn active_enterprise_blocks_authority_removal_until_suspended() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let mandate = fixture.authority.mandate;

    let revoke_error = validate_revoke_mandate(&fixture.state, mandate)
        .expect_err("active routine work must block mandate revocation");
    assert_eq!(
        revoke_error,
        DelegationError::ActiveEnterpriseDependency {
            mandate,
            enterprise,
        }
    );

    let replacement_scope = ResponsibilityScope::Function(ResponsibilityFunction::Finance);
    let revision_error = validate_revise_mandate(
        &fixture.state,
        mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([replacement_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect_err("active routine work must preserve its delegated scope");
    assert_eq!(
        revision_error,
        DelegationError::ActiveEnterpriseScopeDependency {
            mandate,
            enterprise,
            scope: fixture.authority.scope,
        }
    );

    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("enterprise should suspend before authority is removed")
        .commit(&mut fixture.state)
        .expect("enterprise suspension should commit");
    validate_revoke_mandate(&fixture.state, mandate)
        .expect("suspended routine work should release active mandate dependency")
        .commit(&mut fixture.state)
        .expect("mandate revocation should commit after suspension");

    let resume_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("enterprise must not resume under revoked authority"),
        Err(error) => error,
    };
    assert_eq!(
        resume_error,
        EnterpriseError::Delegation(DelegationError::InactiveMandate(mandate))
    );
    validate_invariants(&fixture.state);
}

#[test]
fn save_round_trip_preserves_due_schedule_and_deterministic_cycle_resolution() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    let envelope = build_save(&registry, &fixture.state)
        .expect("active enterprise state should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored =
        restore_save(&registry, decoded).expect("enterprise save should restore cleanly");
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .expect("restored enterprise should exist")
            .next_cycle_at(),
        Some(SimTime::from_minutes(1_440))
    );

    let original_outcome = run_tick(&registry, &mut fixture.state);
    let restored_outcome = run_tick(&registry, &mut restored);
    assert_eq!(original_outcome, restored_outcome);
    assert_eq!(original_outcome.enterprise_cycles.len(), 1);
    let cycle = original_outcome.enterprise_cycles[0];
    let original_cycle = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("original cycle should exist");
    let restored_cycle = restored
        .enterprises()
        .get_cycle(cycle)
        .expect("restored continuation should create the same cycle ID");
    assert_eq!(
        original_cycle.gross_revenue(),
        restored_cycle.gross_revenue()
    );
    assert_eq!(
        original_cycle.operating_cost(),
        restored_cycle.operating_cost()
    );
    assert_eq!(original_cycle.net_cash(), restored_cycle.net_cash());
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("original cash account should exist")
            .balance(),
        restored
            .finance()
            .get_account(fixture.cash)
            .expect("restored cash account should exist")
            .balance()
    );
    validate_invariants(&fixture.state);
    validate_invariants(&restored);
}

#[test]
fn financial_reporting_drills_down_without_cached_totals() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    for variance in [0, 700] {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, variance)
            .expect("due cycle should resolve for reporting fixture");
        validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("reporting fixture cycle should validate")
            .commit(&mut fixture.state)
            .expect("reporting fixture cycle should commit");
    }

    let period_start = SimTime::ZERO;
    let period_end = fixture.state.now();
    let enterprise_summary =
        resolve_enterprise_financial_summary(&fixture.state, enterprise, period_start, period_end)
            .expect("enterprise financial summary should resolve");
    let organization_summary = resolve_organization_enterprise_financial_summary(
        &fixture.state,
        fixture.organization,
        period_start,
        period_end,
    )
    .expect("organization financial summary should resolve");
    let manager_summary = resolve_manager_enterprise_financial_summary(
        &fixture.state,
        fixture.authority.manager,
        period_start,
        period_end,
    )
    .expect("manager financial summary should resolve");
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let neighborhood_summary = resolve_neighborhood_enterprise_financial_summary(
        &fixture.state,
        neighborhood,
        period_start,
        period_end,
    )
    .expect("neighborhood financial summary should resolve");

    assert_eq!(enterprise_summary.totals.enterprise_count, 1);
    assert_eq!(enterprise_summary.totals.cycle_count, 2);
    assert_eq!(enterprise_summary.totals.notable_cycle_count, 1);
    assert_eq!(enterprise_summary.totals, organization_summary.totals);
    assert_eq!(enterprise_summary.totals, manager_summary.totals);
    assert_eq!(enterprise_summary.totals, neighborhood_summary.totals);
    assert_eq!(
        enterprise_summary
            .by_kind
            .get(&EnterpriseKind::Protection)
            .expect("protection bucket should exist"),
        &enterprise_summary.totals
    );
    let cycle_net = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .try_fold(Money::ZERO, |total, cycle| {
            total.checked_add(cycle.net_cash())
        })
        .expect("reporting fixture total should not overflow");
    assert_eq!(enterprise_summary.totals.net_cash, cycle_net);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should exist")
            .balance(),
        enterprise_summary.totals.net_cash
    );
    validate_invariants(&fixture.state);
}

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
fn autonomous_expansion_serves_governed_rivals_and_never_the_player_organization() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let player = designate_player(&registry, &mut fixture.state);

    // The rival's mandate covers the district; the player organization has no mandate at all,
    // so even the designated player cannot receive autonomous establishments here.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
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

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    assert!(apply_due_autonomous_enterprises(&registry, &mut fixture.state).is_empty());
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state).len(),
        1,
        "the pass fires exactly on the day boundary"
    );
}

#[test]
fn autonomous_expansion_rotates_kinds_and_hosts_the_rival_venue() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();

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
    let first_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
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
        EnterpriseKind::Protection,
        "authored kind order puts asset-free rackets first"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
    assert_eq!(second_day.len(), 1);
    // The asset-free kind fills its district slot first, then hosts at the owned venue.
    let second_kind = {
        let second_probe = fixture
            .state
            .enterprises()
            .get_enterprise(second_day[0])
            .expect("day-two enterprise should persist");
        (second_probe.kind(), second_probe.location())
    };
    assert_eq!(second_kind.0, EnterpriseKind::Protection);
    assert!(matches!(second_kind.1, EnterpriseLocation::Business(_)));
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let third_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
    assert_eq!(third_day.len(), 1);
    let third = fixture
        .state
        .enterprises()
        .get_enterprise(third_day[0])
        .expect("day-three enterprise should persist");
    assert_eq!(third.kind(), EnterpriseKind::Gambling);
    match third.location() {
        EnterpriseLocation::Business(host) => {
            let host_name = fixture
                .state
                .world()
                .get_business(host)
                .expect("hosted venue should exist")
                .name()
                .to_owned();
            assert_eq!(host_name, "Rival Card Room");
        }
        other => panic!("gambling must host at the venue, got {other:?}"),
    }

    // Each establishment reserved its own exclusive settlement account.
    assert_ne!(first_kind.2, third.settlement_account());
    validate_invariants(&fixture.state);
}

#[test]
fn police_fear_above_the_authored_ceiling_stalls_expansion_until_it_cools() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
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
        apply_due_autonomous_enterprises(&registry, &mut fixture.state).is_empty(),
        "an outfit above the fear ceiling must keep its head down"
    );

    // Once the impression decays back under the ceiling the same mandate expands again.
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
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
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
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state);
    assert_eq!(established.len(), 1);
    let location = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("establishment should persist")
        .location();
    assert_eq!(
        location,
        EnterpriseLocation::Neighborhood(led),
        "consolidation preference must outrank raw district id order"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn chronic_losing_enterprise_reports_losses_then_suspends_at_the_authored_threshold() {
    let registry = build_registry();
    let mut state = AppState::new(0xBEEF_5105);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Bleeding Outfit".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Dead Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(5),
                    commercial_activity: rating(5),
                    illicit_demand: rating(5),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(95),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    // A manager with no Management capability earns no management revenue premium.
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Unskilled Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager fixture should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate fixture should validate")
    .commit(&mut state)
    .expect("mandate fixture should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("cash account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    let enterprise = crate::enterprises::enterprise_execution::validate_establish_enterprise(
        &registry,
        &state,
        crate::enterprises::EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization,
            authority: MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .expect("enterprise should establish")
    .commit(&mut state)
    .expect("enterprise should commit");

    let threshold = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .losing_cycles_before_suspension() as usize;
    let mut last_information = None;
    for cycle_index in 0..threshold {
        state.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(&registry, &state, enterprise, 0)
            .expect("losing enterprise cycle should decide");
        assert!(
            plan.net_cash().cents() < 0,
            "fixture must produce a losing settlement"
        );
        assert_eq!(plan.attention(), AttentionClass::Notable);
        let cycle = validate_enterprise_cycle_plan(&state, plan)
            .expect("losing cycle plan should validate")
            .commit(&mut state)
            .expect("losing cycle should commit");
        let record = state
            .enterprises()
            .get_cycle(cycle)
            .expect("cycle record should persist");
        last_information = record.information();
        if cycle_index + 1 < threshold {
            assert_eq!(
                state
                    .enterprises()
                    .get_enterprise(enterprise)
                    .map(|record| record.status()),
                Some(crate::enterprises::EnterpriseStatus::Active),
                "racket stays active below the suspension threshold"
            );
        }
    }
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should persist");
    assert_eq!(
        record.status(),
        crate::enterprises::EnterpriseStatus::Suspended
    );
    assert_eq!(record.next_cycle_at(), None);
    let information = state
        .intelligence()
        .get_information(last_information.expect("notable losing cycle should report"))
        .expect("manager report should persist");
    assert!(information.summary().contains("suspended"));

    crate::enterprises::enterprise_execution::validate_resume_enterprise(
        &registry, &state, enterprise,
    )
    .expect("suspended racket should resume")
    .commit(&mut state)
    .expect("resumed racket should commit");
    assert_eq!(
        state
            .enterprises()
            .get_enterprise(enterprise)
            .map(|record| record.status()),
        Some(crate::enterprises::EnterpriseStatus::Active)
    );
    crate::core::invariants::validate_invariants(&state);
}
