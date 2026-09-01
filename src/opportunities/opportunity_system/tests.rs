//! Focused tests for opportunity discovery, conversion, dismissal, and expiry.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::SimDuration;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_system::{
    OperationTransition, apply_transition, validate_authorize_operation,
};
use crate::operations::{OperationApproach, OperationDraft, OperationObjective, RoleKind};
use crate::opportunities::OpportunityResolution;
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_organization,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, OrganizationDraft,
};
use std::collections::{BTreeMap, BTreeSet};

struct OpportunityFixture {
    registry: Registry,
    state: AppState,
    organization: OrganizationId,
    business: crate::core::id::BusinessId,
    leader: crate::core::id::CharacterId,
    entry_specialist: crate::core::id::CharacterId,
    source: InformationId,
}

fn make_fixture() -> OpportunityFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x0F90_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Opportunity Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal organization fixture should validate");
    let neighborhood = crate::world::world_system::insert_neighborhood(
        &mut state,
        crate::world::NeighborhoodDraft {
            name: "Bellmore Ward".to_owned(),
            profile: crate::world::NeighborhoodProfile {
                economy: crate::world::NeighborhoodEconomyProfile {
                    wealth: crate::world::Rating::try_new(70).unwrap(),
                    commercial_activity: crate::world::Rating::try_new(70).unwrap(),
                    illicit_demand: crate::world::Rating::try_new(40).unwrap(),
                },
                institutions: crate::world::NeighborhoodInstitutionProfile {
                    police_presence: crate::world::Rating::try_new(45).unwrap(),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Bellmore Jewelry".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business fixture should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Opportunity Crew Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let entry_specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Opportunity Entry Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Burglary,
                crate::world::Rating::try_new(70).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("entry specialist fixture should validate");
    let source = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::Informant,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Business(business),
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::General,
            summary:
                "A jewelry delivery is expected on Thursday and the night security appears light."
                    .to_owned(),
        },
    )
    .expect("opportunity source information should validate")
    .commit(&mut state)
    .expect("opportunity source information should commit");
    OpportunityFixture {
        registry,
        state,
        organization,
        business,
        leader,
        entry_specialist,
        source,
    }
}

fn opportunity_draft(
    fixture: &OpportunityFixture,
    valid_until: SimTime,
) -> OperationOpportunityDraft {
    OperationOpportunityDraft {
        organization: fixture.organization,
        operation_kind: OperationKind::Burglary,
        targets: BTreeSet::from([EntityRef::Business(fixture.business)]),
        source_information: BTreeSet::from([fixture.source]),
        summary: "Bellmore Jewelry may be vulnerable around its Thursday delivery window."
            .to_owned(),
        valid_until: Some(valid_until),
    }
}

fn authorize_operation(
    fixture: &mut OpportunityFixture,
    objective: OperationObjective,
) -> OperationId {
    validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Bellmore Jewelry burglary".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: fixture.organization,
            leader: fixture.leader,
            objective,
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, fixture.leader),
                (RoleKind::EntrySpecialist, fixture.entry_specialist),
            ]),
            intelligence: BTreeSet::from([fixture.source]),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::from_minutes(10),
        },
    )
    .expect("matching opportunity operation should validate")
    .commit(&mut fixture.state)
    .expect("matching opportunity operation should commit")
}

fn authorize_matching_operation(fixture: &mut OpportunityFixture) -> OperationId {
    authorize_operation(
        fixture,
        OperationObjective::AcquireProperty {
            target: EntityRef::Business(fixture.business),
        },
    )
}

#[test]
fn discovery_requires_organization_knowledge_and_creates_a_provenance_report() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(120)),
    )
    .expect("organization-held target information should support an opportunity")
    .commit(&mut fixture.state)
    .expect("validated opportunity discovery should commit");

    let record = fixture
        .state
        .opportunities()
        .get_opportunity(opportunity)
        .expect("opportunity should persist");
    assert_eq!(record.status(), OpportunityStatus::Open);
    assert_eq!(
        record.source_information(),
        &BTreeSet::from([fixture.source])
    );
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .map(OpportunityRecord::context),
        Some(record.context())
    );
    assert_eq!(
        fixture
            .state
            .opportunities()
            .opportunity_for_report(record.report())
            .map(OpportunityRecord::id),
        Some(opportunity)
    );
    let report = fixture
        .state
        .reports()
        .get_report(record.report())
        .expect("opportunity discovery report should persist");
    assert_eq!(report.kind(), ReportKind::Opportunity);
    assert_eq!(report.recipient(), fixture.organization);
    assert_eq!(report.entries().len(), 1);
    assert_eq!(report.entries()[0].sources, vec![fixture.source]);
    assert!(
        report.entries()[0]
            .entities
            .contains(&EntityRef::Business(fixture.business))
    );
    validate_state(&fixture.state).expect("discovered opportunity state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn duplicate_open_opportunity_is_rejected_but_dismissal_allows_later_rediscovery() {
    let mut fixture = make_fixture();
    let draft = opportunity_draft(&fixture, SimTime::from_minutes(120));
    let opportunity =
        validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft.clone())
            .expect("first opportunity should validate")
            .commit(&mut fixture.state)
            .expect("first opportunity should commit");
    assert_eq!(
        validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft.clone())
            .err()
            .expect("duplicate open opportunity should fail"),
        OpportunityError::ExistingOpenOpportunity(opportunity)
    );
    validate_dismiss_opportunity(&fixture.state, opportunity)
        .expect("open opportunity should be dismissible")
        .commit(&mut fixture.state)
        .expect("dismissal should commit");
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("dismissed opportunity should persist")
            .status(),
        OpportunityStatus::Dismissed
    );
    let replacement =
        validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft)
            .expect("dismissed opportunity should not block a later rediscovery")
            .commit(&mut fixture.state)
            .expect("replacement opportunity should commit");
    assert_ne!(replacement, opportunity);
    validate_invariants(&fixture.state);
}

#[test]
fn multi_target_discovery_converts_against_one_of_its_targets() {
    let mut fixture = make_fixture();
    // A second covered target: the fixture leader's presence in the situation.
    let owner_intel = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Character(fixture.leader),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::General,
            summary: "The property's watchman is a known fixture personality.".to_owned(),
        },
    )
    .expect("second target information should validate")
    .commit(&mut fixture.state)
    .expect("second target information should commit");
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        OperationOpportunityDraft {
            targets: BTreeSet::from([
                EntityRef::Business(fixture.business),
                EntityRef::Character(fixture.leader),
            ]),
            source_information: BTreeSet::from([fixture.source, owner_intel]),
            ..opportunity_draft(&fixture, SimTime::from_minutes(120))
        },
    )
    .expect("multi-target opportunity should validate when every target is covered")
    .commit(&mut fixture.state)
    .expect("multi-target opportunity should commit");

    // The operation acts against only one of the discovered targets.
    let operation = authorize_matching_operation(&mut fixture);
    validate_convert_opportunity(&fixture.state, opportunity, operation)
        .expect("operation targeting one discovered target should convert the opportunity")
        .commit(&mut fixture.state)
        .expect("validated conversion should commit");
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("converted opportunity should persist")
            .status(),
        OpportunityStatus::Converted
    );
    validate_invariants(&fixture.state);
}

#[test]
fn conversion_requires_exact_authorized_operation_and_survives_save_round_trip() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(120)),
    )
    .expect("opportunity should validate")
    .commit(&mut fixture.state)
    .expect("opportunity should commit");
    let operation = authorize_matching_operation(&mut fixture);

    validate_convert_opportunity(&fixture.state, opportunity, operation)
        .expect("matching authorized operation should convert the opportunity")
        .commit(&mut fixture.state)
        .expect("validated opportunity conversion should commit");
    let record = fixture
        .state
        .opportunities()
        .get_opportunity(opportunity)
        .expect("converted opportunity should persist");
    assert_eq!(record.status(), OpportunityStatus::Converted);
    assert_eq!(
        record
            .resolution()
            .and_then(OpportunityResolution::operation),
        Some(operation)
    );
    assert_eq!(
        fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .map(OpportunityRecord::id),
        Some(opportunity)
    );

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("converted opportunity should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&fixture.registry, decoded)
        .expect("converted opportunity save should restore");
    assert_eq!(
        restored
            .opportunities()
            .opportunity_for_operation(operation)
            .map(OpportunityRecord::id),
        Some(opportunity)
    );
    validate_state(&restored).expect("restored opportunity state should validate");
    validate_invariants(&restored);
}

#[test]
fn opportunity_expiry_runs_in_stable_tick_pipeline_and_releases_duplicate_key() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(2)),
    )
    .expect("short-lived opportunity should validate")
    .commit(&mut fixture.state)
    .expect("short-lived opportunity should commit");
    let first = run_tick(&fixture.registry, &mut fixture.state);
    assert!(first.expired_opportunities.is_empty());
    let second = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(second.expired_opportunities, vec![opportunity]);
    let record = fixture
        .state
        .opportunities()
        .get_opportunity(opportunity)
        .expect("expired opportunity should remain historical");
    assert_eq!(record.status(), OpportunityStatus::Expired);
    let expiry_report = match record.resolution() {
        Some(OpportunityResolution::Expired { at, report }) => {
            assert_eq!(at, SimTime::from_minutes(2));
            report
        }
        resolution => panic!("expected expired opportunity, found {resolution:?}"),
    };
    let report = fixture
        .state
        .reports()
        .get_report(expiry_report)
        .expect("opportunity expiry report should persist");
    assert_eq!(report.kind(), ReportKind::Opportunity);
    assert_eq!(report.generated_at(), SimTime::from_minutes(2));
    assert_eq!(report.entries().len(), 1);
    assert_eq!(
        report.entries()[0].summary,
        "Opportunity expired: Bellmore Jewelry may be vulnerable around its Thursday delivery window."
    );
    assert_eq!(report.entries()[0].sources, vec![fixture.source]);
    assert_eq!(
        fixture
            .state
            .opportunities()
            .opportunity_for_report(expiry_report)
            .map(OpportunityRecord::id),
        Some(opportunity)
    );

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("expired opportunity should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored =
        restore_save(&fixture.registry, decoded).expect("expired opportunity save should restore");
    let restored_record = restored
        .opportunities()
        .get_opportunity(opportunity)
        .expect("expired opportunity should survive save/load");
    assert_eq!(restored_record.status(), OpportunityStatus::Expired);
    assert_eq!(
        restored_record
            .resolution()
            .and_then(OpportunityResolution::report),
        Some(expiry_report)
    );
    assert_eq!(
        restored
            .opportunities()
            .opportunity_for_report(expiry_report)
            .map(OpportunityRecord::id),
        Some(opportunity)
    );
    validate_state(&restored).expect("restored expired opportunity state should validate");
    validate_invariants(&restored);

    validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(120)),
    )
    .expect("expired opportunity should release its duplicate key")
    .commit(&mut fixture.state)
    .expect("replacement opportunity should commit");
    validate_state(&fixture.state).expect("expired opportunity state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn expiry_report_reaches_later_executive_brief_after_discovery_window_has_closed() {
    let mut fixture = make_fixture();
    designate_player_organization(&mut fixture.state, fixture.organization)
        .expect("criminal fixture organization should be eligible as player organization");
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(2_880)),
    )
    .expect("two-day opportunity should validate")
    .commit(&mut fixture.state)
    .expect("two-day opportunity should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    let first = run_tick(&fixture.registry, &mut fixture.state);
    let first_brief = first
        .executive_brief
        .expect("first daily boundary should create an executive brief");
    assert!(
        fixture
            .state
            .reports()
            .get_report(first_brief)
            .expect("first executive brief should persist")
            .entries()
            .iter()
            .any(|entry| entry.summary
                == "Bellmore Jewelry may be vulnerable around its Thursday delivery window.")
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    let second = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(second.expired_opportunities, vec![opportunity]);
    let second_brief = second
        .executive_brief
        .expect("second daily boundary should create an executive brief");
    let entries = fixture
        .state
        .reports()
        .get_report(second_brief)
        .expect("second executive brief should persist")
        .entries();
    assert!(entries.iter().any(|entry| {
    entry.summary
      == "Opportunity expired: Bellmore Jewelry may be vulnerable around its Thursday delivery window."
  }));
    assert!(!entries.iter().any(|entry| {
        entry.summary == "Bellmore Jewelry may be vulnerable around its Thursday delivery window."
    }));
    validate_state(&fixture.state)
        .expect("expiry-report executive-brief integration should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn expiry_token_rejects_clock_staleness_without_partial_report_mutation() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(2)),
    )
    .expect("short-lived opportunity should validate")
    .commit(&mut fixture.state)
    .expect("short-lived opportunity should commit");
    fixture.state.advance_clock(SimDuration::from_minutes(2));
    let expiry = validate_expire_opportunity(&fixture.registry, &fixture.state, opportunity)
        .expect("due opportunity expiry should validate");
    let report_count_before = fixture
        .state
        .reports()
        .reports_for(fixture.organization)
        .count();
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);

    assert_eq!(
        expiry
            .commit(&mut fixture.state)
            .expect_err("clock movement must stale a validated expiry transaction"),
        OpportunityError::StaleExpiryTime {
            expected: SimTime::from_minutes(2),
            found: SimTime::from_minutes(3),
        }
    );
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("stale expiry must preserve opportunity")
            .status(),
        OpportunityStatus::Open
    );
    assert_eq!(
        fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .count(),
        report_count_before
    );

    let expiry_report = validate_expire_opportunity(&fixture.registry, &fixture.state, opportunity)
        .expect("overdue opportunity should support a fresh expiry transaction")
        .commit(&mut fixture.state)
        .expect("fresh overdue expiry should commit atomically");
    let record = fixture
        .state
        .opportunities()
        .get_opportunity(opportunity)
        .expect("fresh expiry should preserve historical opportunity");
    assert_eq!(record.status(), OpportunityStatus::Expired);
    assert_eq!(
        record.resolution(),
        Some(OpportunityResolution::Expired {
            at: SimTime::from_minutes(2),
            report: expiry_report,
        })
    );
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(expiry_report)
            .expect("fresh expiry report should persist")
            .generated_at(),
        SimTime::from_minutes(3)
    );
    validate_state(&fixture.state).expect("fresh overdue expiry should restore valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn conversion_token_rejects_operation_lifecycle_change_without_mutating_opportunity() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(120)),
    )
    .expect("opportunity should validate")
    .commit(&mut fixture.state)
    .expect("opportunity should commit");
    let operation = authorize_matching_operation(&mut fixture);
    let conversion = validate_convert_opportunity(&fixture.state, opportunity, operation)
        .expect("fresh conversion should validate");
    fixture.state.advance_clock(SimDuration::from_minutes(10));
    apply_transition(
        &fixture.registry,
        &mut fixture.state,
        operation,
        OperationTransition::Begin,
    )
    .expect("operation should begin after conversion validation");

    let error = conversion
        .commit(&mut fixture.state)
        .expect_err("started operation must stale the older conversion token");
    assert!(matches!(
      error,
      OpportunityError::StaleOperation { operation: id, .. } if id == operation
    ));
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("stale conversion must leave opportunity present")
            .status(),
        OpportunityStatus::Open
    );
    assert!(
        fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .is_none()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn discovery_rejects_personal_and_foreign_knowledge_without_partial_mutation() {
    let mut fixture = make_fixture();
    let personal_source = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Character(fixture.leader),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Business(fixture.business),
            observed_at: fixture.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "The crew leader personally observed the rear service entrance.".to_owned(),
        },
    )
    .expect("personal source fixture should validate")
    .commit(&mut fixture.state)
    .expect("personal source fixture should commit");
    let foreign_organization = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Foreign Information Holder".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("foreign organization fixture should validate");
    let foreign_source = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(foreign_organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Business(fixture.business),
            observed_at: fixture.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "Another organization mapped the jewelry store's rear entrance.".to_owned(),
        },
    )
    .expect("foreign source fixture should validate")
    .commit(&mut fixture.state)
    .expect("foreign source fixture should commit");

    for unavailable in [personal_source, foreign_source] {
        let mut draft = opportunity_draft(&fixture, SimTime::from_minutes(120));
        draft.source_information = BTreeSet::from([unavailable]);
        assert_eq!(
            validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft,)
                .err()
                .expect("non-organizational knowledge must not support opportunity discovery"),
            OpportunityError::InformationUnavailable {
                information: unavailable,
                organization: fixture.organization,
            }
        );
    }
    assert_eq!(
        fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .count(),
        0
    );
    validate_state(&fixture.state)
        .expect("rejected opportunity discovery must leave structurally valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn conversion_rejects_mismatched_operation_kind_without_consuming_opportunity() {
    let mut fixture = make_fixture();
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(120)),
    )
    .expect("burglary opportunity should validate")
    .commit(&mut fixture.state)
    .expect("burglary opportunity should commit");
    let operation = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Bellmore Jewelry scouting".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.organization,
            leader: fixture.leader,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Business(fixture.business),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, fixture.leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::from_minutes(10),
        },
    )
    .expect("mismatched operation fixture should still be independently valid")
    .commit(&mut fixture.state)
    .expect("mismatched operation fixture should commit");

    assert_eq!(
        validate_convert_opportunity(&fixture.state, opportunity, operation)
            .err()
            .expect("wrong operation kind must not consume opportunity"),
        OpportunityError::OperationKindMismatch {
            operation,
            operation_kind: OperationKind::Surveillance,
            opportunity_kind: OperationKind::Burglary,
        }
    );
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("rejected conversion must preserve opportunity")
            .status(),
        OpportunityStatus::Open
    );
    assert!(
        fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .is_none()
    );
    validate_state(&fixture.state).expect("mismatched conversion rejection must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn operations_without_property_effects_reject_property_objectives() {
    let fixture = make_fixture();
    let error = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Invalid intimidation seizure".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: fixture.organization,
            leader: fixture.leader,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(fixture.business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, fixture.leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now(),
        },
    )
    .expect_err("an operation without a property effect must reject property acquisition");

    assert_eq!(
        error,
        crate::operations::operation_system::OperationError::InvalidObjectiveForKind {
            kind: OperationKind::Intimidation,
            objective: crate::operations::OperationObjectiveKind::AcquireProperty,
        }
    );
    assert_eq!(fixture.state.operations().operations().count(), 0);
    validate_state(&fixture.state)
        .expect("rejected property objective must leave structurally valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn opportunity_report_flows_into_next_executive_brief() {
    let mut fixture = make_fixture();
    designate_player_organization(&mut fixture.state, fixture.organization)
        .expect("criminal fixture organization should be eligible as player organization");
    let summary = "Bellmore Jewelry may be vulnerable around its Thursday delivery window.";
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(2_000)),
    )
    .expect("player opportunity should validate")
    .commit(&mut fixture.state)
    .expect("player opportunity should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    let tick = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(tick.now, SimTime::from_minutes(1_440));
    assert!(tick.expired_opportunities.is_empty());
    let executive_brief = tick
        .executive_brief
        .expect("daily boundary should synthesize the opportunity report");
    let brief = fixture
        .state
        .reports()
        .get_report(executive_brief)
        .expect("executive brief should persist");
    assert_eq!(brief.kind(), ReportKind::ExecutiveBrief);
    assert!(brief.entries().iter().any(|entry| {
        entry.attention == AttentionClass::Notable
            && entry.summary == summary
            && entry
                .entities
                .contains(&EntityRef::Business(fixture.business))
    }));
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("long-lived opportunity should remain open")
            .status(),
        OpportunityStatus::Open
    );
    validate_state(&fixture.state)
        .expect("executive-brief opportunity integration should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn converted_opportunity_discovery_is_not_resurfaced_in_later_executive_brief() {
    let mut fixture = make_fixture();
    designate_player_organization(&mut fixture.state, fixture.organization)
        .expect("criminal fixture organization should be eligible as player organization");
    let summary = "Bellmore Jewelry may be vulnerable around its Thursday delivery window.";
    let opportunity = validate_discover_operation_opportunity(
        &fixture.registry,
        &fixture.state,
        opportunity_draft(&fixture, SimTime::from_minutes(2_000)),
    )
    .expect("player opportunity should validate")
    .commit(&mut fixture.state)
    .expect("player opportunity should commit");
    let operation = authorize_matching_operation(&mut fixture);
    validate_convert_opportunity(&fixture.state, opportunity, operation)
        .expect("matching operation should convert the opportunity")
        .commit(&mut fixture.state)
        .expect("opportunity conversion should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    let tick = run_tick(&fixture.registry, &mut fixture.state);
    let executive_brief = tick
        .executive_brief
        .expect("daily boundary should synthesize an executive brief");
    let brief = fixture
        .state
        .reports()
        .get_report(executive_brief)
        .expect("executive brief should persist");
    assert!(brief.entries().iter().all(|entry| entry.summary != summary));
    assert_eq!(
        fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("converted opportunity should persist")
            .status(),
        OpportunityStatus::Converted
    );
    validate_state(&fixture.state)
        .expect("converted-opportunity brief filtering should remain structurally valid");
    validate_invariants(&fixture.state);
}
