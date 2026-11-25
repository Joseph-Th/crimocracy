//! Global trained-crew matching, including overlapping specialties where the
//! old scarcest-role greedy selection stranded a feasible last role.

use super::planning::optimal_staff_match;
use super::*;

fn id(number: u32) -> CharacterId {
    // Typed persistent IDs are allocated by world creation, not minted from
    // arbitrary raw numbers in the external text-play adapter.
    static ROSTER: std::sync::OnceLock<[CharacterId; 4]> = std::sync::OnceLock::new();
    let roster = ROSTER.get_or_init(|| {
        let registry = crimocracy::build_registry();
        let session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
        session
            .state
            .world()
            .characters_in_organization(session.player)
            .map(|member| member.id())
            .collect::<Vec<_>>()
            .try_into()
            .expect("the authored opening has four current members")
    });
    roster[usize::try_from(number - 1).unwrap()]
}

#[test]
fn global_matching_finds_a_feasible_crew_when_greedy_consumes_the_only_flexible_member() {
    // All three required roles have two trained candidates. A greedy plan
    // chooses char-1 for role 0 and char-3 for role 1, then role 2 has no one.
    // Reserving char-2 for role 0 allows a complete assignment instead.
    let candidates = vec![
        (id(1), vec![Some(90), Some(80), Some(80)]),
        (id(2), vec![Some(70), None, None]),
        (id(3), vec![None, Some(70), Some(70)]),
    ];
    let assigned = optimal_staff_match(3, &candidates)
        .expect("feasible three-role matching must not be rejected");
    assert_eq!(assigned, vec![id(2), id(1), id(3)]);
    assert_eq!(assigned.iter().copied().collect::<BTreeSet<_>>().len(), 3);
}

#[test]
fn maximum_total_proficiency_wins_and_equal_sums_break_ties_by_role_ordered_ids() {
    let candidates = vec![
        (id(3), vec![Some(100), Some(5)]),
        (id(1), vec![Some(90), Some(50)]),
        (id(2), vec![Some(60), Some(10)]),
    ];
    // char-3 on role 0 and char-1 on role 1 yields 150, greater
    // than char-1 on role 0 and char-3 on role 1 (95).
    assert_eq!(
        optimal_staff_match(2, &candidates),
        Some(vec![id(3), id(1)])
    );
    let equal = vec![
        (id(4), vec![Some(40), Some(40)]),
        (id(1), vec![Some(40), Some(40)]),
        (id(2), vec![Some(40), Some(40)]),
    ];
    let expected = Some(vec![id(1), id(2)]);
    assert_eq!(optimal_staff_match(2, &equal), expected);
    assert_eq!(
        optimal_staff_match(2, &equal.into_iter().rev().collect::<Vec<_>>()),
        expected,
        "role selection must not depend on roster insertion order"
    );
}

#[test]
fn matching_does_not_fabricate_untrained_people_or_reuse_a_member() {
    let candidates = vec![
        (id(1), vec![Some(100), Some(100), Some(100)]),
        (id(2), vec![Some(20), Some(20), Some(20)]),
        (id(3), vec![None, None, None]),
    ];
    assert_eq!(optimal_staff_match(3, &candidates), None);
    assert_eq!(optimal_staff_match(0, &candidates), Some(Vec::new()));
    assert_eq!(optimal_staff_match(1, &candidates), Some(vec![id(1)]));
    assert_eq!(optimal_staff_match(2, &[]), None);
}

#[test]
fn person_target_is_excluded_from_automatic_crew_and_leadership() {
    use crimocracy::world::world_system::insert_character;
    use crimocracy::world::{AutonomyLevel, CapabilityKind, CharacterDraft, Rating};

    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let target = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| person.capability(CapabilityKind::Surveillance).is_some())
        .unwrap()
        .id();
    let fallback = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Alternative scout".to_owned(),
            organization: Some(session.player),
            supervisor: Some(id(2)),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(
                CapabilityKind::Surveillance,
                Rating::try_new(1).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let preview = session
        .execute(&format!("preview surveillance char-{}", target.raw()))
        .unwrap()
        .0;
    assert!(preview.contains("Alternative scout"), "{preview}");
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );

    // Explicitly assigning the forbidden person is still rejected; do not silently replace them.
    for assignment in ["Surveillance", "leader"] {
        assert!(
            session
                .execute(&format!(
                    "plan surveillance char-{} {assignment}=char-{}",
                    target.raw(),
                    target.raw()
                ))
                .is_err()
        );
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
        );
    }
    session
        .execute(&format!("plan surveillance char-{}", target.raw()))
        .unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|operation| operation.kind() == OperationKind::Surveillance)
        .unwrap();
    assert_eq!(operation.leader(), fallback);
    assert_eq!(operation.roles()[&RoleKind::Surveillance], fallback);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&session.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn optional_authored_roles_are_available_to_manual_plans() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let scout = id(4);
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let command = format!("hijacking business-1 Lookout=char-{}", scout.raw());
    let preview = session.execute(&format!("preview {command}")).unwrap().0;
    assert!(preview.contains("Lookout:"), "{preview}");
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );

    let catalog = session.execute("operation-kinds").unwrap().0;
    let hijacking = catalog
        .lines()
        .find(|line| line.starts_with("Hijacking |"))
        .unwrap();
    assert!(hijacking.contains("optional roles:"), "{hijacking}");
    assert!(hijacking.contains("Lookout (Surveillance)"), "{hijacking}");
    for invalid in [
        format!(
            "plan hijacking business-1 SafeSpecialist=char-{}",
            scout.raw()
        ),
        format!(
            "plan hijacking business-1 Lookout=char-{} Driver=char-{}",
            scout.raw(),
            scout.raw()
        ),
    ] {
        assert!(session.execute(&invalid).is_err(), "{invalid}");
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
        );
    }

    session.execute(&format!("plan {command}")).unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|op| op.kind() == OperationKind::Hijacking)
        .unwrap();
    assert_eq!(operation.roles()[&RoleKind::Lookout], scout);
    assert_eq!(
        operation.roles().len(),
        registry
            .get_operation(OperationKind::Hijacking)
            .required_roles()
            .len()
            + 1
    );
    assert_eq!(
        operation
            .roles()
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        operation.roles().len()
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&session.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn manual_role_assignment_reduces_the_auto_matching_pool_without_override() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let receipt = session
        .execute("plan hijacking business-1 Driver=char-4")
        .unwrap()
        .0;
    assert!(receipt.contains("Authorized Hijacking"), "{receipt}");
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|op| op.kind() == OperationKind::Hijacking)
        .unwrap();
    assert_eq!(operation.roles().get(&RoleKind::Driver), Some(&id(3)));
    assert_ne!(operation.roles().get(&RoleKind::Coordinator), Some(&id(3)));
    let saved = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
