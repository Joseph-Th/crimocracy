//! Extraction planning, custody effects, and episode-bound persistence regressions.

use super::*;

fn record_known_detention_status(
    state: &mut AppState,
    organization: OrganizationId,
    detainee: CharacterId,
    arrest: crate::core::id::ArrestId,
) {
    validate_record_information_with_signal(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: None,
            subject: EntityRef::Character(detainee),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "The organization confirmed this person is currently detained.".to_owned(),
        },
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained { arrest }),
    )
    .expect("typed detention-status knowledge should validate")
    .commit(state)
    .expect("typed detention-status knowledge should commit");
}

#[test]
fn extraction_from_stale_foreign_detention_report_aborts_without_invalidating_save() {
    let registry = build_registry();
    let mut state = AppState::new(1933);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Rescue crew".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Rescue precinct".into(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .unwrap();
    let mut member = |name: &str, organization, supervisor| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.into(),
                organization,
                supervisor,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Management, Rating::try_new(99).unwrap()),
                    (CapabilityKind::Driving, Rating::try_new(99).unwrap()),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap()
    };
    let leader = member("Rescue leader", Some(crew), None);
    let driver = member("Rescue driver", Some(crew), Some(leader));
    let outsider = member("Independent detainee", None, None);
    let arrest =
        detain_pressure_test_character(&registry, &mut state, police, outsider, "Outsider custody");
    state.advance_clock(SimDuration::ONE_MINUTE);
    record_known_detention_status(&mut state, crew, outsider, arrest);
    let mut successful = state.clone();
    crate::legal::arrest_system::validate_release_arrest(&state, arrest)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    state.advance_clock(SimDuration::ONE_MINUTE);
    // The crew's information is still usable, and a foreign early release is
    // not ambient knowledge. Planning must not expose it as a free oracle.
    let draft = OperationDraft {
        title: "Extraction based on recent report".into(),
        kind: OperationKind::Extraction,
        responsible_organization: crew,
        leader,
        objective: OperationObjective::FreeDetainee { target: outsider },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: state.now() + SimDuration::ONE_MINUTE,
    };
    // A successful rescue also establishes release for an outsider, without
    // requiring them to join the organization or consulting private custody.
    let rescue = validate_authorize_operation(&registry, &successful, draft.clone())
        .unwrap()
        .commit(&mut successful)
        .unwrap();
    run_until_operation_resolved(&registry, &mut successful, rescue);
    assert_ne!(
        successful
            .operations()
            .get_operation(rescue)
            .unwrap()
            .resolution()
            .unwrap()
            .objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(
        successful.legal().get_arrest(arrest).unwrap().status(),
        crate::legal::ArrestStatus::Released
    );
    let repeat = OperationDraft {
        scheduled_for: successful.now() + SimDuration::ONE_MINUTE,
        ..draft.clone()
    };
    assert_eq!(
        validate_authorize_operation(&registry, &successful, repeat).err(),
        Some(OperationError::TargetLegalBasisUnknown {
            kind: OperationKind::Extraction,
            character: outsider
        })
    );
    restore_save(&registry, build_save(&registry, &successful).unwrap()).unwrap();

    let operation = validate_authorize_operation(&registry, &state, draft.clone())
        .unwrap()
        .commit(&mut state)
        .unwrap();
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .unwrap()
            .extraction_arrest(),
        Some(arrest)
    );
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = run_tick(&registry, &mut state);
    assert_eq!(tick, run_tick(&registry, &mut restored));
    assert_eq!(tick.aborted_operations, vec![operation]);
    assert!(tick.started_operations.is_empty());
    let record = state.operations().get_operation(operation).unwrap();
    assert_eq!(
        record.abort_record().unwrap().cause(),
        OperationAbortCause::ObjectiveUnavailable(
            OperationObjectiveBlocker::ExtractionCustodyEnded
        )
    );
    assert!(record.started_at().is_none());
    assert_eq!(
        state.legal().get_arrest(arrest).unwrap().status(),
        crate::legal::ArrestStatus::Released
    );
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    for current in [&mut state, &mut restored] {
        let before = bincode::serialize(current).unwrap();
        let repeat = OperationDraft {
            scheduled_for: current.now() + SimDuration::ONE_MINUTE,
            ..draft.clone()
        };
        assert_eq!(
            validate_authorize_operation(&registry, current, repeat.clone()).err(),
            Some(OperationError::TargetLegalBasisUnknown {
                kind: OperationKind::Extraction,
                character: outsider,
            }),
            "the crew's own after-action report confirms that this arrest ended"
        );
        assert_eq!(bincode::serialize(current).unwrap(), before);
        // A new episode does not become known merely by existing. Only a fresh
        // report can license another extraction, and it pins the new arrest.
        let renewed = detain_pressure_test_character(
            &registry,
            current,
            police,
            outsider,
            "Renewed outsider custody",
        );
        assert_ne!(renewed, arrest);
        assert!(validate_authorize_operation(&registry, current, repeat.clone()).is_err());
        record_known_detention_status(current, crew, outsider, renewed);
        let next = validate_authorize_operation(&registry, current, repeat)
            .unwrap()
            .commit(current)
            .unwrap();
        assert_eq!(
            current
                .operations()
                .get_operation(next)
                .unwrap()
                .extraction_arrest(),
            Some(renewed)
        );
    }
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
}

#[test]
fn successful_extraction_frees_detained_member_through_canonical_release() {
    let registry = build_registry();
    let mut state = AppState::new(0x0E77_1933);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Extraction Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Extraction Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let mut make_member = |name: &str, supervisor: Option<CharacterId>| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(crew),
                supervisor,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (
                        CapabilityKind::Management,
                        Rating::try_new(99).expect("fixture rating should be valid"),
                    ),
                    (
                        CapabilityKind::Driving,
                        Rating::try_new(99).expect("fixture rating should be valid"),
                    ),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("member should validate")
    };
    let leader = make_member("Extraction Leader", None);
    let driver = make_member("Extraction Driver", Some(leader));
    let detainee = make_member("Detained Member", Some(leader));

    // Put the member in custody through the canonical evidence-backed arrest path.
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Detention test case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(detainee)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("evidence should validate")
    .commit(&mut state)
    .expect("evidence should commit");
    let corroborating = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating evidence should validate")
    .commit(&mut state)
    .expect("corroborating evidence should commit");
    let arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("evidence-backed arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");
    let unknown_custody_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Unknown custody extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("hidden detention must not be queryable through extraction authorization");
    assert_eq!(
        unknown_custody_error,
        OperationError::TargetLegalBasisUnknown {
            kind: OperationKind::Extraction,
            character: detainee,
        }
    );
    record_known_detention_status(&mut state, crew, detainee, arrest);

    // A free-detainee objective against someone not in custody must be rejected.
    let free_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Impossible extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: leader },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect_err("extraction requires a detained target");
    assert!(matches!(
      free_error,
      OperationError::TargetLegalBasisUnknown {
          kind: OperationKind::Extraction,
          character,
      } if character == leader
    ));

    // A plan that cannot finish before the current custody window ends is knowingly obsolete
    // before it starts and must be rejected rather than waiting for a due-tick failure.
    let maximum_detention = registry.legal().maximum_detention();
    let extraction_duration = registry
        .get_operation(OperationKind::Extraction)
        .execution()
        .duration();
    let impossible_start = SimTime::from_minutes(
        state.now().as_minutes()
            + u64::from(maximum_detention.as_minutes() - extraction_duration.as_minutes() + 1),
    );
    let impossible_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Too-late extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: impossible_start,
        },
    )
    .expect_err("an extraction cannot be planned beyond the target's current custody");
    assert!(matches!(
        impossible_error,
        OperationError::ExtractionMissesCustodyWindow {
            character,
            planned_end,
            custody_ends_at,
        } if character == detainee
            && planned_end == impossible_start + extraction_duration
            && custody_ends_at == state.now() + maximum_detention
    ));

    // Completion at the release instant is also too late: the custody relationship ends at that
    // instant, so the operation must finish strictly before it rather than racing the release pass.
    let boundary_start = SimTime::from_minutes(
        state.now().as_minutes()
            + u64::from(maximum_detention.as_minutes() - extraction_duration.as_minutes()),
    );
    let boundary_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Boundary extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: boundary_start,
        },
    )
    .expect_err("an extraction must finish before the custody release instant");
    assert!(matches!(
        boundary_error,
        OperationError::ExtractionMissesCustodyWindow {
            character,
            planned_end,
            custody_ends_at,
        } if character == detainee
            && planned_end == boundary_start + extraction_duration
            && custody_ends_at == state.now() + maximum_detention
    ));

    // A custody relationship can also end early after authorization but before the crew enters.
    // The due operation must cancel before start rather than spending an execution window on a
    // detainee who is already free.
    let mut released_before_start = state.clone();
    let cancelled_extraction = validate_authorize_operation(
        &registry,
        &released_before_start,
        OperationDraft {
            title: "Released-before-entry extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: released_before_start.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("current custody should allow a near-term extraction")
    .commit(&mut released_before_start)
    .expect("near-term extraction should commit");
    crate::legal::arrest_system::validate_release_arrest(&released_before_start, arrest)
        .expect("custody should be releasable before the operation begins")
        .commit(&mut released_before_start)
        .expect("early custody release should commit");
    let cancelled_tick = run_tick(&registry, &mut released_before_start);
    assert!(cancelled_tick.started_operations.is_empty());
    let cancelled = released_before_start
        .operations()
        .get_operation(cancelled_extraction)
        .expect("cancelled extraction should persist");
    assert_eq!(cancelled.status(), OperationStatus::Aborted);
    assert_eq!(
        cancelled.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::ObjectiveUnavailable(
            OperationObjectiveBlocker::ExtractionCustodyEnded,
        ))
    );
    assert!(cancelled.started_at().is_none());
    validate_state_against_registry(&registry, &released_before_start)
        .expect("pre-start custody-loss abort should remain registry-valid");
    validate_invariants(&released_before_start);

    // A corrupted authorization time cannot place a due-start objective abort
    // before the earliest legal execution minute.
    let original = released_before_start
        .operations()
        .get_operation(cancelled_extraction)
        .expect("cancelled extraction should persist")
        .clone();
    let mut corrupted_operation = original.clone();
    corrupted_operation.command.authorized_at = SimTime::from_minutes(1);
    let original_bytes =
        bincode::serialize(&original).expect("cancelled extraction should serialize");
    let corrupted_bytes =
        bincode::serialize(&corrupted_operation).expect("corrupted extraction should serialize");
    assert_eq!(
        corrupted_bytes.len(),
        original_bytes.len(),
        "fixed-width authorization-time corruption must preserve operation wire size"
    );
    let envelope = build_save(&registry, &released_before_start)
        .expect("canonical released-before-start extraction should save");
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "cancelled extraction operation must occur once in the save"
    );
    envelope_bytes[matches[0]..matches[0] + corrupted_bytes.len()]
        .copy_from_slice(&corrupted_bytes);
    let corrupted: SaveEnvelope = bincode::deserialize(&envelope_bytes)
        .expect("same-layout authorization-time corruption should decode");
    assert!(matches!(
        restore_save(&registry, corrupted),
        Err(crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidOperationDefinition {
                operation
            }
        )) if operation == cancelled_extraction
    ));

    // If an already-running extraction is delayed until the custody cap, mandatory release is the
    // first same-minute lifecycle action. The operation then records a practical failure instead
    // of receiving credit for breaking custody that legally ended at the same instant.
    let mut expiry_state = state.clone();
    let expiry_extraction = validate_authorize_operation(
        &registry,
        &expiry_state,
        OperationDraft {
            title: "Expired-custody extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: expiry_state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("early extraction should validate")
    .commit(&mut expiry_state)
    .expect("early extraction should commit");
    let start_outcome = run_tick(&registry, &mut expiry_state);
    assert_eq!(start_outcome.started_operations, vec![expiry_extraction]);
    let release_at = expiry_state
        .legal()
        .get_arrest(arrest)
        .expect("fixture arrest should persist")
        .arrested_at()
        + maximum_detention;
    let minutes_until_pre_release = release_at
        .as_minutes()
        .checked_sub(expiry_state.now().as_minutes() + 1)
        .expect("fresh extraction starts before custody release");
    expiry_state.advance_clock(SimDuration::from_minutes(
        u32::try_from(minutes_until_pre_release).expect("fixture detention window must fit u32"),
    ));
    let expiry_outcome = run_tick(&registry, &mut expiry_state);
    assert_eq!(expiry_outcome.now, release_at);
    assert_eq!(
        expiry_outcome.custody_releases,
        vec![crate::core::simulation::CustodyEvent {
            arrest,
            organization: Some(crew),
        }],
    );
    assert_eq!(expiry_outcome.resolved_operations, vec![expiry_extraction]);
    let expiry_resolution = expiry_state
        .operations()
        .get_operation(expiry_extraction)
        .and_then(|record| record.resolution())
        .expect("expired-custody extraction should still resolve");
    assert_eq!(
        expiry_resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(
        expiry_resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::ExtractionCustodyEnded)
    );
    assert_eq!(expiry_resolution.extraction_arrest(), None);
    assert_eq!(
        expiry_state
            .legal()
            .get_arrest(arrest)
            .and_then(|record| record.released_at()),
        Some(release_at)
    );
    validate_state(&expiry_state).expect("custody-boundary extraction state should validate");
    validate_invariants(&expiry_state);

    let extraction = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Bust-out extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect("detained-target extraction should validate")
    .commit(&mut state)
    .expect("extraction should commit");

    // A second live extraction against the same custody is rejected: it could only
    // resolve after the first freed the target and would then be uncommittable.
    let duplicate_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Duplicate extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect_err("a detainee supports exactly one live extraction plan");
    assert!(matches!(
      duplicate_error,
      OperationError::DetaineeAlreadyTargeted {
        character,
        operation
      } if character == detainee && operation == extraction
    ));
    let operation_count = state.operations().operations().count();
    let outcome = run_until_operation_resolved(&registry, &mut state, extraction);
    assert_eq!(
        outcome.custody_releases,
        vec![crate::core::simulation::CustodyEvent {
            arrest,
            organization: Some(crew),
        }],
        "the canonical operation phase must surface the extraction release exactly once",
    );
    let record = state
        .operations()
        .get_operation(extraction)
        .expect("extraction should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    assert_ne!(
        record
            .resolution()
            .map(|resolution| resolution.objective_outcome()),
        Some(OperationObjectiveOutcome::Failed),
        "a fully capable crew must not fail the extraction"
    );
    assert_eq!(
        record
            .resolution()
            .and_then(|resolution| resolution.extraction_arrest()),
        Some(arrest),
        "successful extraction history must identify the custody relationship it broke"
    );
    let released = state
        .legal()
        .get_arrest(arrest)
        .expect("arrest should persist");
    assert_eq!(
        released.status(),
        crate::legal::ArrestStatus::Released,
        "successful extraction must release the detainee through canonical custody"
    );
    assert!(released.released_at().is_some());
    let before = bincode::serialize(&state).unwrap();
    let repeat = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Redundant rescue".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("the crew already knows its member is free");
    assert_eq!(
        repeat,
        OperationError::TargetLegalBasisUnknown {
            kind: OperationKind::Extraction,
            character: detainee,
        }
    );
    assert_eq!(bincode::serialize(&state).unwrap(), before);
    assert_eq!(
        state.operations().operations().count(),
        operation_count,
        "the rejected duplicate extraction must not create a record"
    );
    validate_state(&state).expect("post-extraction state should remain valid");
    validate_invariants(&state);

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("successful extraction state should save"),
    )
    .expect("successful extraction state should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(extraction)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.extraction_arrest()),
        Some(arrest)
    );
    validate_invariants(&restored);

    // Custody can still end after a valid authorization. If it ends after execution starts, the
    // due resolution must become a practical objective failure, not panic because the release
    // transaction no longer has an arrest to consume. Renewed custody in this same case must be
    // based on evidence discovered after the extraction release.
    state.advance_clock(SimDuration::ONE_MINUTE);
    let renewed_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::CommunicationRecord,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("post-release evidence should validate")
    .commit(&mut state)
    .expect("post-release evidence should commit");
    let rearrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, renewed_evidence]),
        },
    )
    .expect("evidence-backed renewed arrest should validate")
    .commit(&mut state)
    .expect("evidence-backed renewed arrest should commit");
    record_known_detention_status(&mut state, crew, detainee, rearrest);
    let stale_authorization = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Stale custody authorization".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect("fresh custody should produce an extraction authorization token");
    crate::legal::arrest_system::validate_release_arrest(&state, rearrest)
        .expect("first renewed arrest should be releasable before authorization commit")
        .commit(&mut state)
        .expect("first renewed arrest release should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    let replacement_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::FinancialRecord,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("replacement-custody evidence should validate")
    .commit(&mut state)
    .expect("replacement-custody evidence should commit");
    let replacement_arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, replacement_evidence]),
        },
    )
    .expect("replacement custody should validate from newly developed evidence")
    .commit(&mut state)
    .expect("replacement custody should commit");
    record_known_detention_status(&mut state, crew, detainee, replacement_arrest);
    let stale_error = stale_authorization
        .commit(&mut state)
        .expect_err("authorization must not retarget itself to replacement custody");
    assert_eq!(
        stale_error,
        OperationError::StaleExtractionBasis {
            character: detainee,
            expected: rearrest,
            found: Some(replacement_arrest),
        }
    );

    let stale_target_extraction = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Custody-ended extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect("replacement custody should support a fresh extraction")
    .commit(&mut state)
    .expect("fresh extraction should commit");
    assert_eq!(
        state
            .operations()
            .get_operation(stale_target_extraction)
            .and_then(|record| record.extraction_arrest()),
        Some(replacement_arrest),
        "the operation command must freeze the exact custody relationship it targets"
    );
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(
        &registry,
        &mut state,
        stale_target_extraction,
        OperationTransition::Begin,
    )
    .expect("extraction should begin while custody is current");
    crate::legal::arrest_system::validate_release_arrest(&state, replacement_arrest)
        .expect("custody should still be releasable after operation start")
        .commit(&mut state)
        .expect("external custody release should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    let later_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("later-custody evidence should validate")
    .commit(&mut state)
    .expect("later-custody evidence should commit");
    let later_arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, later_evidence]),
        },
    )
    .expect("new evidence makes the later re-arrest a distinct legal event")
    .commit(&mut state)
    .expect("later re-arrest should commit without retargeting the active operation");
    state.advance_clock(SimDuration::from_minutes(
        extraction_duration.as_minutes() - 1,
    ));
    let outcome = run_tick(&registry, &mut state);
    assert_eq!(
        outcome.resolved_operations,
        vec![stale_target_extraction],
        "custody expiry must remain a resolvable canonical tick outcome"
    );
    let stale_resolution = state
        .operations()
        .get_operation(stale_target_extraction)
        .and_then(|record| record.resolution())
        .expect("custody-ended extraction should still persist a resolution");
    assert_eq!(
        stale_resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(stale_resolution.extraction_arrest(), None);
    assert_eq!(
        stale_resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::ExtractionCustodyEnded)
    );
    assert_eq!(
        state
            .legal()
            .active_arrest_for_character(detainee)
            .map(|arrest| arrest.id()),
        Some(later_arrest),
        "the old extraction must not release a newer custody event it was never authorized against"
    );
    let after_action = state
        .intelligence()
        .get_information(stale_resolution.after_action_information())
        .expect("failed extraction should persist an after-action record");
    assert!(after_action.summary().contains("no longer detained"));
    validate_state(&state).expect("custody-ended extraction state should remain valid");
    validate_invariants(&state);
}
