//! Focused tests for world insertion, designation, policies, and membership reassignment.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    StateValidationError, validate_invariants, validate_state_against_registry,
};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::time::{SimDuration, SimTime};
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
use crate::world::{
    ApprovalPolicy, AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
    CharacterDraft, LegalSupportPolicy, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    PolicyKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn make_test_character(
    state: &mut AppState,
    name: &str,
    organization: OrganizationId,
    supervisor: Option<CharacterId>,
) -> CharacterId {
    insert_character(
        state,
        CharacterDraft {
            name: name.to_owned(),
            organization: Some(organization),
            supervisor,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("test character should validate")
}

fn replace_serialized_organization(
    envelope: SaveEnvelope,
    original: &OrganizationRecord,
    replacement: &OrganizationRecord,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("organization should serialize");
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement organization should serialize");
    assert_eq!(original_bytes.len(), replacement_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "organization record should occur once in save"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout organization corruption should remain decodable")
}

#[test]
fn recruitment_policy_history_is_reconstructible_from_toggle_versions() {
    let registry = build_registry();
    let mut state = AppState::new(0x504F_4C48);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Policy History Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");

    let record = state
        .world()
        .get_organization(organization)
        .expect("organization should persist");
    assert_eq!(
        record.independent_recruitment_policy_at_version(1),
        Some(ApprovalPolicy::RequireApproval)
    );

    set_policy(
        &registry,
        &mut state,
        organization,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
    )
    .expect("first real policy change should establish version two");
    set_policy(
        &registry,
        &mut state,
        organization,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    )
    .expect("second real policy change should establish version three");

    let record = state
        .world()
        .get_organization(organization)
        .expect("organization should persist after policy changes");
    assert_eq!(
        record.policy_version(PolicyKind::IndependentRecruitment),
        Some(3)
    );
    assert_eq!(
        record.independent_recruitment_policy_at_version(1),
        Some(ApprovalPolicy::RequireApproval)
    );
    assert_eq!(
        record.independent_recruitment_policy_at_version(2),
        Some(ApprovalPolicy::Delegated)
    );
    assert_eq!(
        record.independent_recruitment_policy_at_version(3),
        Some(ApprovalPolicy::RequireApproval)
    );
    assert_eq!(record.independent_recruitment_policy_at_version(0), None);
    assert_eq!(record.independent_recruitment_policy_at_version(4), None);
    validate_invariants(&state);
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn make_test_business(
    registry: &Registry,
    state: &mut AppState,
    owner: BusinessOwner,
) -> BusinessId {
    let neighborhood = insert_neighborhood(
        state,
        NeighborhoodDraft {
            name: "Ownership Test Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(60),
                    illicit_demand: rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(50),
                },
            },
        },
    )
    .expect("test neighborhood should validate");
    insert_business(
        registry,
        state,
        BusinessDraft {
            name: "Ownership Test Business".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner,
        },
    )
    .expect("test business should validate")
}

#[test]
fn player_organization_designation_is_single_assignment_campaign_identity() {
    let registry = build_registry();
    let mut state = AppState::new(0x504C_4159);
    let first = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Player Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("first criminal organization should validate");
    let second = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Player Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second criminal organization should validate");

    designate_player_organization(&mut state, first)
        .expect("first player organization designation should commit");
    assert_eq!(state.player_organization(), Some(first));

    assert_eq!(
        designate_player_organization(&mut state, second),
        Err(WorldError::PlayerOrganizationAlreadyDesignated {
            current: first,
            requested: second,
        })
    );
    assert_eq!(state.player_organization(), Some(first));
    assert_eq!(
        designate_player_organization(&mut state, first),
        Err(WorldError::PlayerOrganizationAlreadyDesignated {
            current: first,
            requested: first,
        }),
        "designation is a campaign identity assignment, not a repeatable mutation"
    );
    validate_invariants(&state);
}

#[test]
fn restore_rejects_zero_organization_policy_version() {
    let registry = build_registry();
    let mut state = AppState::new(0x504F_4C59);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Versioned Policy Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let original = state
        .world()
        .get_organization(organization)
        .expect("organization fixture should persist")
        .clone();
    let mut corrupted = original.clone();
    corrupted
        .policy_versions
        .insert(PolicyKind::IndependentRecruitment, 0);

    let envelope = build_save(&registry, &state).expect("valid policy state should save");
    let corrupted_envelope = replace_serialized_organization(envelope, &original, &corrupted);

    let error = restore_save(&registry, corrupted_envelope)
        .expect_err("zero organization-policy version must be rejected on restore");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidOrganizationPolicyVersion {
            organization,
            policy: PolicyKind::IndependentRecruitment,
        })
    );
}

#[test]
fn registry_validation_rejects_unreachable_organization_policy_version_history() {
    let registry = build_registry();
    let mut state = AppState::new(0x504F_4C52);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Policy Reachability Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let original = state
        .world()
        .get_organization(organization)
        .expect("organization should persist")
        .clone();
    let mut corrupted = original.clone();
    corrupted
        .policy_versions
        .insert(PolicyKind::IndependentRecruitment, 2);
    let envelope = build_save(&registry, &state).expect("canonical policy state should save");
    let corrupted_envelope = replace_serialized_organization(envelope, &original, &corrupted);
    let error = restore_save(&registry, corrupted_envelope)
        .expect_err("version two cannot retain the version-one recruitment default");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidOrganizationPolicyVersion {
            organization,
            policy: PolicyKind::IndependentRecruitment,
        })
    );

    let mut reachable = original.clone();
    reachable
        .policy_versions
        .insert(PolicyKind::IndependentRecruitment, 3);
    assert_eq!(
        reachable.independent_recruitment_policy_at_version(2),
        Some(ApprovalPolicy::Delegated)
    );
    // The binary policy can return to the authored default at odd versions, so the registry-aware
    // check must reject only unreachable histories rather than treating every version > 1 as bad.
    let valid_envelope = build_save(&registry, &state).expect("canonical state should still save");
    let reachable_envelope = replace_serialized_organization(valid_envelope, &original, &reachable);
    let restored = restore_save(&registry, reachable_envelope)
        .expect("version three with the authored recruitment default is reachable");
    validate_state_against_registry(&registry, &restored)
        .expect("restored reachable policy history should remain registry-valid");
}

#[test]
fn registry_validation_tracks_three_state_legal_support_policy_reachability() {
    let registry = build_registry();
    let mut state = AppState::new(0x504F_4C4C);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Legal Policy Reachability Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let original = state
        .world()
        .get_organization(organization)
        .expect("organization should persist")
        .clone();

    let mut impossible_v2 = original.clone();
    impossible_v2
        .policy_versions
        .insert(PolicyKind::AssociateLegalSupport, 2);
    let envelope = build_save(&registry, &state).expect("canonical policy state should save");
    let impossible_envelope = replace_serialized_organization(envelope, &original, &impossible_v2);
    let error = restore_save(&registry, impossible_envelope)
        .expect_err("version two cannot retain the version-one legal-support default");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidOrganizationPolicyVersion {
            organization,
            policy: PolicyKind::AssociateLegalSupport,
        })
    );

    let mut reachable_v2 = original.clone();
    reachable_v2
        .policy_versions
        .insert(PolicyKind::AssociateLegalSupport, 2);
    reachable_v2.policies.insert(
        PolicyKind::AssociateLegalSupport,
        PolicySetting::AssociateLegalSupport(LegalSupportPolicy::Automatic),
    );
    let envelope = build_save(&registry, &state).expect("canonical policy state should still save");
    let reachable_envelope = replace_serialized_organization(envelope, &original, &reachable_v2);
    let restored = restore_save(&registry, reachable_envelope)
        .expect("version two with a non-default legal-support setting is reachable");
    assert_eq!(
        restored
            .world()
            .get_organization(organization)
            .and_then(|record| record.policy(PolicyKind::AssociateLegalSupport)),
        Some(PolicySetting::AssociateLegalSupport(
            LegalSupportPolicy::Automatic
        ))
    );

    let mut reachable_v3 = original.clone();
    reachable_v3
        .policy_versions
        .insert(PolicyKind::AssociateLegalSupport, 3);
    let envelope = build_save(&registry, &state).expect("canonical policy state should still save");
    let reachable_envelope = replace_serialized_organization(envelope, &original, &reachable_v3);
    restore_save(&registry, reachable_envelope)
        .expect("three-state legal support may return to its authored default by version three");
}

#[test]
fn business_ownership_transfer_updates_indexes_and_preserves_versioned_history() {
    let registry = build_registry();
    let mut state = AppState::new(0x0B51_0001);
    let first_owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Holding Company".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("first owner should validate");
    let second_owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Holding Company".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("second owner should validate");
    let individual_owner =
        make_test_character(&mut state, "Individual Proprietor", second_owner, None);
    let business = make_test_business(
        &registry,
        &mut state,
        BusinessOwner::Organization(first_owner),
    );

    let initial = state
        .world()
        .get_business_ownership_change_for_version(business, 1)
        .expect("initial ownership should be durable");
    assert_eq!(initial.previous_owner(), None);
    assert_eq!(
        initial.new_owner(),
        BusinessOwner::Organization(first_owner)
    );
    assert_eq!(initial.changed_at(), SimTime::ZERO);
    assert_eq!(
        state
            .world()
            .businesses_owned_by_organization(first_owner)
            .count(),
        1
    );

    state.advance_clock(SimDuration::from_minutes(15));
    let transferred = validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Organization(second_owner),
    )
    .expect("ownership transfer should validate")
    .commit(&mut state)
    .expect("ownership transfer should commit");

    let record = state
        .world()
        .get_business(business)
        .expect("business should remain present");
    assert_eq!(record.owner(), BusinessOwner::Organization(second_owner));
    assert_eq!(record.version(), 2);
    assert_eq!(
        state
            .world()
            .businesses_owned_by_organization(first_owner)
            .count(),
        0
    );
    assert_eq!(
        state
            .world()
            .businesses_owned_by_organization(second_owner)
            .count(),
        1
    );
    let change = state
        .world()
        .business_ownership_history(business)
        .find(|record| record.previous_owner().is_some())
        .expect("ownership change should persist");
    assert_eq!(change.id(), transferred);
    assert_eq!(
        change.previous_owner(),
        Some(BusinessOwner::Organization(first_owner))
    );
    assert_eq!(
        change.new_owner(),
        BusinessOwner::Organization(second_owner)
    );
    assert_eq!(change.changed_at(), SimTime::from_minutes(15));
    assert_eq!(change.resulting_business_version(), 2);
    assert_eq!(
        state.world().business_ownership_history(business).count(),
        2
    );
    assert_eq!(
        state.world().business_owner_at(business, SimTime::ZERO),
        Some(BusinessOwner::Organization(first_owner))
    );
    assert_eq!(
        state
            .world()
            .business_owner_at(business, SimTime::from_minutes(15)),
        Some(BusinessOwner::Organization(second_owner))
    );

    state.advance_clock(SimDuration::from_minutes(5));
    validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Character(individual_owner),
    )
    .expect("character ownership transfer should validate")
    .commit(&mut state)
    .expect("character ownership transfer should commit");
    let record = state
        .world()
        .get_business(business)
        .expect("business should remain present after character transfer");
    assert_eq!(record.owner(), BusinessOwner::Character(individual_owner));
    assert_eq!(record.version(), 3);
    assert_eq!(
        state
            .world()
            .businesses_owned_by_organization(second_owner)
            .count(),
        0
    );
    assert_eq!(
        state
            .world()
            .businesses_ever_owned_by_organization(first_owner)
            .count(),
        1
    );
    assert_eq!(
        state
            .world()
            .businesses_ever_owned_by_organization(second_owner)
            .count(),
        1
    );
    assert_eq!(
        state
            .world()
            .businesses_owned_by_character(individual_owner)
            .count(),
        1
    );
    assert_eq!(
        state.world().business_ownership_history(business).count(),
        3
    );
    assert_eq!(
        state
            .world()
            .business_owner_at(business, SimTime::from_minutes(20)),
        Some(BusinessOwner::Character(individual_owner))
    );
    assert!(state.world().has_business_owner_during(
        business,
        BusinessOwner::Organization(second_owner),
        SimTime::from_minutes(15),
        SimTime::from_minutes(20),
    ));
    assert!(!state.world().has_business_owner_during(
        business,
        BusinessOwner::Organization(first_owner),
        SimTime::from_minutes(15),
        SimTime::from_minutes(20),
    ));
    assert!(!state.world().has_business_owner_during(
        business,
        BusinessOwner::Organization(second_owner),
        SimTime::from_minutes(20),
        SimTime::from_minutes(20),
    ));
    assert!(state.world().has_business_owner_during(
        business,
        BusinessOwner::Character(individual_owner),
        SimTime::from_minutes(20),
        SimTime::from_minutes(20),
    ));
    validate_invariants(&state);
}

#[test]
fn stale_business_ownership_token_cannot_overwrite_newer_title() {
    let registry = build_registry();
    let mut state = AppState::new(0x0B51_0002);
    let first_owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Initial Owner".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("first owner should validate");
    let intended_owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Intended Buyer".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("intended owner should validate");
    let business = make_test_business(
        &registry,
        &mut state,
        BusinessOwner::Organization(first_owner),
    );
    let stale = validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Organization(intended_owner),
    )
    .expect("first transfer should validate");
    validate_transfer_business_ownership(&state, business, BusinessOwner::Independent)
        .expect("newer transfer should validate")
        .commit(&mut state)
        .expect("newer transfer should commit");

    let error = stale
        .commit(&mut state)
        .expect_err("stale transfer must not overwrite newer title");
    assert_eq!(
        error,
        WorldError::StaleBusiness {
            business,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .world()
            .get_business(business)
            .expect("business should remain present")
            .owner(),
        BusinessOwner::Independent
    );
    assert_eq!(
        state.world().business_ownership_history(business).count(),
        2
    );
    validate_invariants(&state);
}

#[test]
fn reassignment_rejects_supervision_cycle_without_mutation() {
    let registry = build_registry();
    let mut state = AppState::new(7);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("test organization should validate");
    let boss = make_test_character(&mut state, "Boss", organization, None);
    let lieutenant = make_test_character(&mut state, "Lieutenant", organization, Some(boss));
    let soldier = make_test_character(&mut state, "Soldier", organization, Some(lieutenant));

    let error = validate_reassign_character(&state, boss, Some(organization), Some(soldier))
        .expect_err("cycle must be rejected before mutation");
    assert_eq!(error, WorldError::SupervisionCycle { character: boss });
    assert_eq!(
        state
            .world
            .get_character(boss)
            .expect("boss should still exist")
            .supervisor(),
        None
    );
    assert_eq!(state.world.direct_reports(lieutenant).count(), 1);
    validate_invariants(&state);
}

#[test]
fn reassignment_updates_hierarchy_indexes_atomically() {
    let registry = build_registry();
    let mut state = AppState::new(11);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("test organization should validate");
    let boss = make_test_character(&mut state, "Boss", organization, None);
    let lieutenant = make_test_character(&mut state, "Lieutenant", organization, Some(boss));
    let soldier = make_test_character(&mut state, "Soldier", organization, Some(lieutenant));

    validate_reassign_character(&state, soldier, Some(organization), Some(boss))
        .expect("valid reassignment should produce a token")
        .commit(&mut state)
        .expect("validated reassignment should remain current");

    assert_eq!(state.world.direct_reports(lieutenant).count(), 0);
    assert_eq!(state.world.direct_reports(boss).count(), 2);
    validate_invariants(&state);
}

#[test]
fn unassigned_character_cannot_have_organization_supervisor() {
    let registry = build_registry();
    let mut state = AppState::new(13);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("test organization should validate");
    let supervisor = make_test_character(&mut state, "Supervisor", organization, None);

    let error = insert_character(
        &mut state,
        CharacterDraft {
            name: "Unassigned".to_owned(),
            organization: None,
            supervisor: Some(supervisor),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect_err("unassigned character must not enter an organization hierarchy");

    assert_eq!(
        error,
        WorldError::SupervisorWithoutOrganization { supervisor }
    );
    validate_invariants(&state);
}

#[test]
fn unassigned_character_cannot_have_unassigned_supervisor() {
    let mut state = AppState::new(13);
    let supervisor = insert_character(
        &mut state,
        CharacterDraft {
            name: "Unassigned Supervisor".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unassigned supervisor fixture should validate");

    let error = insert_character(
        &mut state,
        CharacterDraft {
            name: "Unassigned".to_owned(),
            organization: None,
            supervisor: Some(supervisor),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect_err("unassigned character must not enter an organization hierarchy");

    assert_eq!(
        error,
        WorldError::SupervisorWithoutOrganization { supervisor }
    );
    assert_eq!(state.world.direct_reports(supervisor).count(), 0);
    validate_invariants(&state);
}

#[test]
fn stale_reassignment_token_cannot_overwrite_newer_membership() {
    let registry = build_registry();
    let mut state = AppState::new(17);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("test organization should validate");
    let boss = make_test_character(&mut state, "Boss", organization, None);
    let first = make_test_character(&mut state, "First", organization, Some(boss));
    let second = make_test_character(&mut state, "Second", organization, Some(boss));
    let member = make_test_character(&mut state, "Member", organization, Some(first));

    let stale = validate_reassign_character(&state, member, Some(organization), Some(second))
        .expect("first reassignment should validate");
    let current = validate_reassign_character(&state, member, Some(organization), Some(boss))
        .expect("second reassignment should validate against the same snapshot");
    current
        .commit(&mut state)
        .expect("current reassignment should commit");

    let error = stale
        .commit(&mut state)
        .expect_err("stale reassignment must not overwrite newer membership");
    assert_eq!(
        error,
        WorldError::StaleCharacter {
            character: member,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .world()
            .get_character(member)
            .expect("member should exist")
            .supervisor(),
        Some(boss)
    );
    validate_invariants(&state);
}

#[test]
fn unchanged_reassignment_is_rejected_without_bumping_the_version() {
    let registry = build_registry();
    let mut state = AppState::new(29);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("test organization should validate");
    let boss = make_test_character(&mut state, "Boss", organization, None);
    let member = make_test_character(&mut state, "Member", organization, Some(boss));
    let version_before = state
        .world()
        .get_character(member)
        .expect("member should exist")
        .version();

    let error = validate_reassign_character(&state, member, Some(organization), Some(boss))
        .expect_err("no-op reassignment must be rejected");
    assert_eq!(
        error,
        WorldError::CharacterReassignmentUnchanged { character: member }
    );
    assert_eq!(
        state
            .world()
            .get_character(member)
            .expect("member should exist")
            .version(),
        version_before,
        "rejected reassignment must not invalidate outstanding tokens"
    );
    validate_invariants(&state);
}

#[test]
fn reassignment_token_revalidates_new_mandate_dependency_at_commit() {
    let registry = build_registry();
    let mut state = AppState::new(23);
    let first_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("first organization should validate");
    let second_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second organization should validate");
    let manager = make_test_character(&mut state, "Manager", first_organization, None);
    let reassignment =
        validate_reassign_character(&state, manager, Some(second_organization), None)
            .expect("reassignment should initially validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: first_organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate should validate after reassignment token creation")
    .commit(&mut state)
    .expect("mandate should commit");

    let error = reassignment
        .commit(&mut state)
        .expect_err("new active mandate must invalidate the older reassignment token");
    assert_eq!(
        error,
        WorldError::ActiveMandateAssignment {
            character: manager,
            mandate,
        }
    );
    assert_eq!(
        state
            .world()
            .get_character(manager)
            .expect("manager should exist")
            .organization(),
        Some(first_organization)
    );
    validate_invariants(&state);
}

#[test]
fn reassignment_token_revalidates_supervisor_membership_at_commit() {
    let registry = build_registry();
    let mut state = AppState::new(29);
    let first_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("first organization should validate");
    let second_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second organization should validate");
    let supervisor = make_test_character(&mut state, "Future Supervisor", first_organization, None);
    let member = make_test_character(&mut state, "Member", first_organization, None);
    let member_reassignment =
        validate_reassign_character(&state, member, Some(first_organization), Some(supervisor))
            .expect("member reassignment should initially validate");
    validate_reassign_character(&state, supervisor, Some(second_organization), None)
        .expect("supervisor should be movable before gaining direct reports")
        .commit(&mut state)
        .expect("supervisor move should commit");

    let error = member_reassignment
        .commit(&mut state)
        .expect_err("supervisor organization change must invalidate member token");
    assert_eq!(
        error,
        WorldError::SupervisorOrganizationMismatch {
            supervisor,
            organization: Some(first_organization),
        }
    );
    assert_eq!(
        state
            .world()
            .get_character(member)
            .expect("member should exist")
            .supervisor(),
        None
    );
    validate_invariants(&state);
}

#[test]
fn supervisor_cannot_leave_organization_with_direct_reports() {
    let registry = build_registry();
    let mut state = AppState::new(31);
    let first_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("first organization should validate");
    let second_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second organization should validate");
    let supervisor = make_test_character(&mut state, "Supervisor", first_organization, None);
    let direct_report = make_test_character(
        &mut state,
        "Direct Report",
        first_organization,
        Some(supervisor),
    );

    let error = validate_reassign_character(&state, supervisor, Some(second_organization), None)
        .expect_err("supervisor must reassign direct reports before leaving organization");
    assert_eq!(
        error,
        WorldError::DirectReportAssignment {
            character: supervisor,
            direct_report,
        }
    );
    validate_invariants(&state);
}
