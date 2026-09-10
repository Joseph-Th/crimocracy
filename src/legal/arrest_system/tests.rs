//! Focused tests for evidence-threshold arrest validation, custody, and autonomous conversion.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::SimDuration;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::informant_system::{
    validate_establish_informant, validate_record_informant_disclosure,
};
use crate::legal::investigation_system::{
    InvestigationError, InvestigationTransition, validate_add_evidence,
    validate_open_investigation, validate_transition_investigation,
};
use crate::legal::{
    Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InformantDisclosureDraft, InformantDraft, InvestigationDraft,
};
use crate::registry::Registry;
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    police: OrganizationId,
    suspect: CharacterId,
    investigation: InvestigationId,
    evidence: EvidenceId,
}

#[test]
fn autonomous_arrest_prefers_stronger_case_over_earlier_investigation_id() {
    let mut fixture = fixture();
    add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let stronger_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Later stronger custody case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
        },
    )
    .expect("stronger case should validate")
    .commit(&mut fixture.state)
    .expect("stronger case should commit");
    assert!(stronger_case > fixture.investigation);
    for _ in 0..3 {
        add_character_evidence(
            &mut fixture.state,
            fixture.police,
            stronger_case,
            fixture.suspect,
        );
    }

    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("competing arrestable cases should resolve");
    assert_eq!(arrests.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_arrest(arrests[0])
            .expect("autonomous arrest should persist")
            .investigation(),
        stronger_case,
        "evidentiary strength must outrank investigation creation order"
    );
    validate_state(&fixture.state).expect("stronger-case custody state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_arrest_can_cite_developed_evidence_when_primary_source_is_not_custody_grade() {
    use crate::legal::investigation_system::validate_assign_investigator;
    use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
    use crate::legal::{InvestigationWorkDraft, InvestigationWorkFocus, InvestigationWorkKind};
    use crate::world::{CapabilityKind, Rating};

    let mut fixture = fixture();
    let case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Developed corroboration custody case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
        },
    )
    .expect("developed-evidence case should validate")
    .commit(&mut fixture.state)
    .expect("developed-evidence case should commit");
    let source = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("questionable source should remain valid investigative evidence")
    .commit(&mut fixture.state)
    .expect("questionable source should commit");
    let detective = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Development Detective".to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(100).expect("fixture rating should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("development detective should validate");
    validate_assign_investigator(&fixture.state, case, detective)
        .expect("development detective assignment should validate")
        .commit(&mut fixture.state)
        .expect("development detective assignment should commit");
    let work = validate_schedule_investigation_work(
        &fixture.registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: case,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(source),
        },
    )
    .expect("questionable source review should schedule")
    .commit(&mut fixture.state)
    .expect("questionable source review should commit");
    loop {
        let tick = run_tick(&fixture.registry, &mut fixture.state);
        if tick.resolved_investigation_work.contains(&work) {
            break;
        }
    }
    let derived = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .and_then(|work| work.resolution())
        .and_then(|resolution| resolution.derived_evidence())
        .expect("high-skill review should develop qualifying forensic evidence");
    let developed = fixture
        .state
        .legal()
        .get_evidence(derived)
        .expect("developed forensic evidence should persist");
    assert_eq!(developed.strength(), EvidenceStrength::Corroborating);
    assert_eq!(developed.reliability(), EvidenceReliability::Mixed);
    assert!(!evidence_qualifies_for_custody(
        fixture
            .state
            .legal()
            .get_evidence(source)
            .expect("primary source should persist")
    ));
    assert!(evidence_qualifies_for_custody(developed));
    let strong = add_character_evidence(&mut fixture.state, fixture.police, case, fixture.suspect);

    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("developed corroboration should produce a valid autonomous arrest");
    assert_eq!(arrests.len(), 1);
    let arrest = fixture
        .state
        .legal()
        .get_arrest(arrests[0])
        .expect("developed-evidence arrest should persist");
    assert_eq!(arrest.investigation(), case);
    assert_eq!(arrest.evidence(), &BTreeSet::from([derived, strong]));
    assert!(!arrest.evidence().contains(&source));
    validate_state(&fixture.state).expect("developed-evidence custody state should validate");
    validate_invariants(&fixture.state);
}

fn add_two_same_source_informant_statements(fixture: &mut Fixture) -> BTreeSet<EvidenceId> {
    let criminal = fixture
        .state
        .world()
        .get_character(fixture.suspect)
        .and_then(|record| record.organization())
        .expect("suspect fixture should belong to a criminal organization");
    let source = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Single Confidential Source".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("confidential-source fixture should validate");
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: source,
            handler: fixture.police,
        },
    )
    .expect("informant relationship should validate")
    .commit(&mut fixture.state)
    .expect("informant relationship should commit");

    let mut statement_evidence = BTreeSet::new();
    for (topic, summary) in [
        (
            InformationTopic::Personnel,
            "The source identifies the suspect through personnel knowledge.",
        ),
        (
            InformationTopic::OperationalOutcome,
            "The source independently describes the suspect's operational activity.",
        ),
    ] {
        let information = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(source),
                source_kind: InformationSourceKind::DirectObservation,
                topic,
                source_entity: None,
                subject: EntityRef::Character(fixture.suspect),
                observed_at: fixture.state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: summary.to_owned(),
            },
        )
        .expect("source information should validate")
        .commit(&mut fixture.state)
        .expect("source information should commit");
        let disclosure = validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        )
        .expect("relevant source disclosure should validate")
        .commit(&mut fixture.state)
        .expect("source disclosure should commit");
        let evidence = fixture
            .state
            .legal()
            .informant_disclosures()
            .find(|record| record.id() == disclosure)
            .expect("source disclosure should persist")
            .evidence();
        statement_evidence.insert(evidence);
    }
    assert_eq!(statement_evidence.len(), 2);
    statement_evidence
}

#[test]
fn repeated_statements_from_one_named_source_count_as_one_corroborator() {
    let mut fixture = fixture();
    let mut statement_evidence = add_two_same_source_informant_statements(&mut fixture);

    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: statement_evidence.clone(),
        },
    )
    .expect_err("two statements from one named source must remain one corroborator");
    assert_eq!(
        error,
        ArrestError::InsufficientIndependentEvidence {
            found: 1,
            required: fixture
                .registry
                .legal()
                .minimum_arrest_qualifying_evidence(),
        }
    );

    statement_evidence.insert(fixture.evidence);
    validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: statement_evidence,
        },
    )
    .expect("one named source plus one independent primary fact should satisfy corroboration");
    validate_state(&fixture.state).expect("same-source disclosure state should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_arrest_with_duplicate_named_corroboration_source() {
    let mut fixture = fixture();
    let statement_evidence = add_two_same_source_informant_statements(&mut fixture);
    let first_statement = *statement_evidence
        .iter()
        .next()
        .expect("same-source fixture should contain statement evidence");
    let arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence, first_statement]),
        },
    )
    .expect("one named source plus one primary fact should support a valid arrest")
    .commit(&mut fixture.state)
    .expect("valid mixed-source arrest should commit");

    let original = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("valid arrest should persist")
        .clone();
    let mut corrupted = original.clone();
    corrupted.evidence = statement_evidence;
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid mixed-source custody state should save before corruption");
    let original_bytes = bincode::serialize(&original).expect("arrest record should serialize");
    let corrupted_bytes =
        bincode::serialize(&corrupted).expect("corrupted arrest should serialize");
    assert_eq!(original_bytes.len(), corrupted_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "arrest record should occur once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + corrupted_bytes.len()].copy_from_slice(&corrupted_bytes);
    let corrupted_envelope: SaveEnvelope = bincode::deserialize(&envelope_bytes)
        .expect("same-layout arrest corruption should remain decodable");

    let error = restore_save(&fixture.registry, corrupted_envelope)
        .expect_err("restore must enforce the same named-source corroboration rule as runtime");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidArrest { arrest: invalid }
        ) if invalid == arrest
    ));
}

#[test]
fn direct_arrest_requires_authored_independent_corroboration() {
    let fixture = fixture();
    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence]),
        },
    )
    .expect_err("one strong fact must not bypass the authored custody corroboration bar");
    assert_eq!(
        error,
        ArrestError::InsufficientIndependentEvidence {
            found: 1,
            required: fixture
                .registry
                .legal()
                .minimum_arrest_qualifying_evidence(),
        }
    );
}

#[test]
fn registry_validation_rejects_detention_persisted_past_maximum_custody() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let maximum_detention = fixture.registry.legal().maximum_detention();
    fixture
        .state
        .set_now_for_test(fixture.state.now() + maximum_detention + SimDuration::ONE_MINUTE);

    validate_state(&fixture.state)
        .expect("release-safe structure alone cannot know the authored custody duration");
    assert!(matches!(
        validate_state_against_registry(&fixture.registry, &fixture.state),
        Err(crate::core::invariants::StateValidationError::InvalidArrest { arrest: invalid })
            if invalid == arrest
    ));
    assert!(matches!(
        build_save(&fixture.registry, &fixture.state),
        Err(crate::core::persistence::SaveError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidArrest { arrest: invalid }
        )) if invalid == arrest
    ));
}

#[test]
fn registry_validation_rejects_release_recorded_after_maximum_custody() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let maximum_detention = fixture.registry.legal().maximum_detention();
    fixture
        .state
        .set_now_for_test(fixture.state.now() + maximum_detention + SimDuration::ONE_MINUTE);
    validate_release_arrest(&fixture.state, arrest)
        .expect("structural release validation permits an explicit release at the current instant")
        .commit(&mut fixture.state)
        .expect("late fixture release should commit before registry-aware validation");

    validate_state(&fixture.state)
        .expect("late released custody is structurally coherent without authored timing");
    assert!(matches!(
        validate_state_against_registry(&fixture.registry, &fixture.state),
        Err(crate::core::invariants::StateValidationError::InvalidArrest { arrest: invalid })
            if invalid == arrest
    ));
}

#[test]
fn custody_release_horizon_overflow_clamps_to_last_representable_minute() {
    let mut fixture = fixture();
    let maximum_detention = fixture.registry.legal().maximum_detention();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(maximum_detention.as_minutes()) + 1,
    ));
    let arrest = arrest_fixture(&mut fixture);

    let released = apply_due_custody_releases(&mut fixture.state, maximum_detention)
        .expect("custody before the clamped horizon should remain active");
    assert!(released.is_empty());

    fixture
        .state
        .set_now_for_test(SimTime::from_minutes(u64::MAX));
    assert_eq!(
        apply_due_custody_releases(&mut fixture.state, maximum_detention)
            .expect("last representable minute must release clamped custody"),
        vec![arrest]
    );
    let record = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("released arrest should remain persisted");
    assert_eq!(record.status(), ArrestStatus::Released);
    assert_eq!(record.released_at(), Some(SimTime::from_minutes(u64::MAX)));
    validate_invariants(&fixture.state);
}

#[derive(Clone, Serialize)]
struct EvidenceIdentityWire {
    id: EvidenceId,
    investigation: InvestigationId,
    custodian: OrganizationId,
}

#[derive(Clone, Serialize)]
struct EvidenceConnectionWire {
    subject: EntityRef,
    origin: Option<EntityRef>,
    source: Option<EntityRef>,
    derived_from: BTreeSet<EvidenceId>,
}

#[derive(Clone, Copy, Serialize)]
struct EvidenceAssessmentWire {
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
}

#[derive(Clone, Serialize)]
struct EvidenceRecordWire {
    identity: EvidenceIdentityWire,
    connection: EvidenceConnectionWire,
    assessment: EvidenceAssessmentWire,
    discovered_at: SimTime,
}

fn evidence_wire(record: &crate::legal::EvidenceRecord) -> EvidenceRecordWire {
    EvidenceRecordWire {
        identity: EvidenceIdentityWire {
            id: record.id(),
            investigation: record.investigation(),
            custodian: record.custodian(),
        },
        connection: EvidenceConnectionWire {
            subject: record.subject(),
            origin: record.origin(),
            source: record.source(),
            derived_from: record.derived_from().clone(),
        },
        assessment: EvidenceAssessmentWire {
            kind: record.kind(),
            strength: record.strength(),
            reliability: record.reliability(),
            admissibility: record.admissibility(),
        },
        discovered_at: record.discovered_at(),
    }
}

fn replace_serialized_evidence(
    envelope: SaveEnvelope,
    original: &crate::legal::EvidenceRecord,
    replacement: &EvidenceRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("evidence record should serialize");
    let mirror = evidence_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("evidence mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement evidence should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized evidence must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout evidence corruption must remain decodable")
}

#[test]
fn restore_rejects_arrest_backed_by_nonqualifying_evidence() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let original = fixture
        .state
        .legal()
        .get_evidence(fixture.evidence)
        .expect("arrest evidence should persist");
    let mut corrupted = evidence_wire(original);
    corrupted.assessment.reliability = EvidenceReliability::Questionable;
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid custody state should save before corruption");
    let corrupted = replace_serialized_evidence(envelope, original, &corrupted);

    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("restore must reject custody supported by evidence canonical arrest rejects");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidArrest { arrest: invalid }
        ) if invalid == arrest
    ));
}

#[test]
fn arrest_rejects_questionable_evidence_even_when_strong() {
    let mut fixture = fixture();
    let questionable = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.suspect),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("questionable material can remain part of an active investigation")
    .commit(&mut fixture.state)
    .expect("questionable material should persist as investigative evidence");

    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([questionable]),
        },
    )
    .expect_err("questionable evidence must not justify custody merely because it is strong");
    assert_eq!(
        error,
        ArrestError::InsufficientEvidence {
            evidence: questionable,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Questionable,
        }
    );
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_arrest_excludes_questionable_material_from_corroboration() {
    let mut fixture = fixture();
    let questionable = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.suspect),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("questionable surveillance should remain valid investigative material")
    .commit(&mut fixture.state)
    .expect("questionable surveillance should persist");

    assert!(
        apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
            .expect("questionable evidence should be ignored rather than failing the pass")
            .is_empty(),
        "one reliable item plus one questionable item does not satisfy corroboration"
    );

    let corroborating = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("two independently usable items should resolve to custody");
    assert_eq!(arrests.len(), 1);
    let arrest = fixture
        .state
        .legal()
        .get_arrest(arrests[0])
        .expect("autonomous arrest should persist");
    assert_eq!(
        arrest.evidence(),
        &BTreeSet::from([fixture.evidence, corroborating]),
        "questionable material stays in the case graph but is not cited as custody support"
    );
    assert!(!arrest.evidence().contains(&questionable));
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_arrest_is_evidence_driven_not_case_origin_driven() {
    let mut fixture = fixture();
    let second = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );

    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("qualifying institution-authored evidence should resolve to custody");
    assert_eq!(arrests.len(), 1);
    let record = fixture
        .state
        .legal()
        .get_arrest(arrests[0])
        .expect("autonomous arrest should persist");
    assert_eq!(record.character(), fixture.suspect);
    assert_eq!(record.investigation(), fixture.investigation);
    assert_eq!(
        record.evidence(),
        &BTreeSet::from([fixture.evidence, second])
    );
    validate_state(&fixture.state).expect("evidence-driven custody state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_arrest_leaves_legal_authority_cases_outside_police_custody() {
    let mut fixture = fixture();
    let legal_authority = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Municipal Investigative Authority".to_owned(),
            kind: OrganizationKind::LegalAuthority,
        },
    )
    .expect("legal authority should validate");
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: legal_authority,
            title: "Administrative corruption file".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
        },
    )
    .expect("legal-authority investigation should validate")
    .commit(&mut fixture.state)
    .expect("legal-authority investigation should commit");
    for kind in [EvidenceKind::Document, EvidenceKind::FinancialRecord] {
        validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation,
                custodian: legal_authority,
                subject: EntityRef::Character(fixture.suspect),
                origin: None,
                kind,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("legal-authority evidence should validate")
        .commit(&mut fixture.state)
        .expect("legal-authority evidence should commit");
    }

    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("non-police investigative evidence must not break the autonomous custody pass");
    assert!(arrests.is_empty());
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );
    validate_state(&fixture.state).expect("legal-authority case should leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn custody_cancels_scheduled_investigation_work_with_arrest_provenance() {
    use crate::legal::investigation_system::validate_assign_investigator;
    use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
    use crate::legal::{
        InvestigationWorkCancellationReason, InvestigationWorkDraft, InvestigationWorkFocus,
        InvestigationWorkKind, InvestigationWorkStatus,
    };
    use crate::world::{CapabilityKind, Rating};

    let mut fixture = fixture();
    let second_authority = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Internal Affairs Authority".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second authority should validate");
    let detective = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Investigated Detective".to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(85).expect("investigation capability should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective should validate");
    let witness_subject = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Separate Case Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("separate case subject should validate");
    let work_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Detective workload case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(witness_subject)]),
        },
    )
    .expect("work case should validate")
    .commit(&mut fixture.state)
    .expect("work case should commit");
    let work_evidence = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        work_case,
        witness_subject,
    );
    validate_assign_investigator(&fixture.state, work_case, detective)
        .expect("detective assignment should validate")
        .commit(&mut fixture.state)
        .expect("detective assignment should commit");
    let work = validate_schedule_investigation_work(
        &fixture.registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: work_case,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::Evidence(work_evidence),
        },
    )
    .expect("detective work should validate")
    .commit(&mut fixture.state)
    .expect("detective work should commit");
    let work_case_activity_before_detention = fixture
        .state
        .legal()
        .get_investigation(work_case)
        .expect("work case should persist")
        .last_activity_at();
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(60));

    let arrest_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: second_authority,
            title: "Detective corruption case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(detective)]),
        },
    )
    .expect("arrest case should validate")
    .commit(&mut fixture.state)
    .expect("arrest case should commit");
    let arrest_evidence =
        add_character_evidence(&mut fixture.state, second_authority, arrest_case, detective);
    let arrest_corroboration =
        add_character_evidence(&mut fixture.state, second_authority, arrest_case, detective);
    let detained_at = fixture.state.now();
    let arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: detective,
            investigation: arrest_case,
            evidence: BTreeSet::from([arrest_evidence, arrest_corroboration]),
        },
    )
    .expect("scheduled detective work should not immunize its owner from custody")
    .commit(&mut fixture.state)
    .expect("custody should cancel scheduled work atomically");

    let work_record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("cancelled work should remain historical");
    assert_eq!(work_record.status(), InvestigationWorkStatus::Cancelled);
    assert!(work_record.resolution().is_none());
    let cancellation = work_record
        .cancellation()
        .expect("cancelled work should retain custody provenance");
    assert_eq!(cancellation.cancelled_at(), detained_at);
    assert_eq!(
        cancellation.reason(),
        InvestigationWorkCancellationReason::InvestigatorDetained(arrest)
    );
    assert!(
        fixture
            .state
            .legal()
            .work_for_investigator(detective)
            .all(|record| record.status() != InvestigationWorkStatus::Scheduled)
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(work_case)
            .expect("active case should persist")
            .lead_investigator(),
        None,
        "custody must release the detective's active lead seat"
    );
    assert!(
        fixture
            .state
            .legal()
            .active_investigations_without_lead()
            .any(|investigation| investigation == work_case),
        "detention-preempted case must be available for institutional restaffing"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(work_case)
            .expect("detention-preempted case should persist")
            .last_activity_at(),
        work_case_activity_before_detention,
        "unrelated detective custody must not manufacture fresh investigative activity"
    );
    validate_state(&fixture.state).expect("work-cancellation custody state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn custody_preflights_shared_case_version_budget_for_work_cancel_and_lead_release() {
    use crate::legal::investigation_system::validate_assign_investigator;
    use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
    use crate::legal::{InvestigationWorkDraft, InvestigationWorkFocus, InvestigationWorkKind};
    use crate::world::{CapabilityKind, Rating};

    let mut fixture = fixture();
    let detective = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Capacity Detective".to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(85).expect("investigation capability should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective should validate");
    let subject = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Capacity Case Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("case subject should validate");
    let case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Capacity workload case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(subject)]),
        },
    )
    .expect("work case should validate")
    .commit(&mut fixture.state)
    .expect("work case should commit");
    let evidence = add_character_evidence(&mut fixture.state, fixture.police, case, subject);
    validate_assign_investigator(&fixture.state, case, detective)
        .expect("detective assignment should validate")
        .commit(&mut fixture.state)
        .expect("detective assignment should commit");
    let work = validate_schedule_investigation_work(
        &fixture.registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: case,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::Evidence(evidence),
        },
    )
    .expect("detective work should validate")
    .commit(&mut fixture.state)
    .expect("detective work should commit");
    let arrest_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Capacity detective custody case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(detective)]),
        },
    )
    .expect("detective custody case should validate")
    .commit(&mut fixture.state)
    .expect("detective custody case should commit");
    let arrest_evidence =
        add_character_evidence(&mut fixture.state, fixture.police, arrest_case, detective);
    let arrest_corroboration =
        add_character_evidence(&mut fixture.state, fixture.police, arrest_case, detective);

    fixture
        .state
        .legal
        .investigations
        .get_mut(&case)
        .expect("work case should persist")
        .version = u32::MAX - 1;
    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: detective,
            investigation: arrest_case,
            evidence: BTreeSet::from([arrest_evidence, arrest_corroboration]),
        },
    )
    .expect_err("custody must reject before two case-version advances exceed capacity");
    let ArrestError::VersionCapacity(error) = error else {
        panic!("unexpected custody capacity error: {error:?}");
    };
    assert_eq!(error.record_kind(), "investigation");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(case)
            .expect("rejected capacity preflight must preserve the case")
            .version(),
        u32::MAX - 1
    );
    assert!(
        fixture
            .state
            .legal()
            .get_investigation_work(work)
            .is_some_and(
                |record| record.status() == crate::legal::InvestigationWorkStatus::Scheduled
            )
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(case)
            .expect("rejected arrest must preserve work-case staffing")
            .lead_investigator(),
        Some(detective)
    );
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(detective)
            .is_none()
    );
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0xA22E_5701);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Test Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Test Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let suspect = insert_character(
        &mut state,
        CharacterDraft {
            name: "Case Subject".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("suspect should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Evidence-backed custody test".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(suspect)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let evidence = add_character_evidence(&mut state, police, investigation, suspect);
    Fixture {
        registry,
        state,
        police,
        suspect,
        investigation,
        evidence,
    }
}

fn add_character_evidence(
    state: &mut AppState,
    police: OrganizationId,
    investigation: InvestigationId,
    suspect: CharacterId,
) -> EvidenceId {
    validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(state)
    .expect("case evidence should commit")
}

fn arrest_fixture(fixture: &mut Fixture) -> ArrestId {
    let corroborating = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence, corroborating]),
        },
    )
    .expect("evidence-backed arrest should validate")
    .commit(&mut fixture.state)
    .expect("evidence-backed arrest should commit")
}

#[test]
fn arrest_and_release_are_durable_indexed_lifecycle_records() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let record = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("arrest should persist");
    assert_eq!(record.status(), ArrestStatus::Detained);
    assert_eq!(record.version(), 1);
    assert_eq!(record.authority(), fixture.police);
    assert_eq!(record.evidence().len(), 2);
    assert!(record.evidence().contains(&fixture.evidence));
    let arrest_evidence = record.evidence().clone();
    assert_eq!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(arrest)
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_for_investigation(fixture.investigation)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("detention state should validate");
    validate_invariants(&fixture.state);

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("detention state should build a save envelope");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&fixture.registry, decoded)
        .expect("detention state should restore with indexes intact");
    assert_eq!(
        restored
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(arrest)
    );
    validate_release_arrest(&restored, arrest)
        .expect("restored detention should remain releasable")
        .commit(&mut restored)
        .expect("restored detention release should commit");
    let rearrest = validate_arrest(
        &fixture.registry,
        &restored,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: arrest_evidence,
        },
    )
    .expect("released restored character should permit a later evidence-backed arrest")
    .commit(&mut restored)
    .expect("later restored arrest should commit with a fresh ID");
    assert_ne!(rearrest, arrest);
    assert_eq!(
        restored
            .legal()
            .arrests()
            .filter(|record| record.character() == fixture.suspect)
            .count(),
        2
    );
    assert_eq!(
        restored
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(rearrest)
    );
    validate_state(&restored).expect("restored re-arrest state should validate");
    validate_invariants(&restored);

    fixture.state.advance_clock(SimDuration::from_minutes(45));
    validate_release_arrest(&fixture.state, arrest)
        .expect("active detention should release")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let released = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("released arrest history should persist");
    assert_eq!(released.status(), ArrestStatus::Released);
    assert_eq!(released.version(), 2);
    assert_eq!(released.released_at(), Some(fixture.state.now()));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests()
            .filter(|record| record.character() == fixture.suspect)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("released custody history should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn due_custody_release_bounds_detention_at_the_authored_window() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let maximum_detention = fixture.registry.legal().maximum_detention();

    fixture.state.advance_clock(SimDuration::from_minutes(
        maximum_detention.as_minutes() - 1,
    ));
    assert!(
        apply_due_custody_releases(&mut fixture.state, maximum_detention)
            .expect("pre-deadline custody pass should resolve")
            .is_empty(),
        "detention must remain active until the complete authored window elapses"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_arrest(arrest)
            .map(|record| record.status()),
        Some(ArrestStatus::Detained)
    );

    // The release deadline is derived from persisted arrest time plus the authored legal
    // definition, so restoring one minute before the cap must preserve the exact next-tick
    // release rather than resetting or extending custody.
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("pre-release custody should build a save envelope");
    let bytes = bincode::serialize(&envelope).expect("custody save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("custody save should deserialize");
    fixture.state = restore_save(&fixture.registry, decoded)
        .expect("pre-release custody should restore with its deadline intact");

    let outcome = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(
        outcome.custody_releases,
        vec![arrest],
        "the canonical minute pipeline must surface the authored custody release"
    );
    let record = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("released arrest remains durable history");
    assert_eq!(record.status(), ArrestStatus::Released);
    assert_eq!(record.released_at(), Some(fixture.state.now()));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );
    validate_state(&fixture.state).expect("bounded custody release state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_custody_does_not_rearrest_a_released_subject_from_the_same_case() {
    let mut fixture = fixture();
    let corroborating = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("two independent strong items should produce autonomous custody");
    assert_eq!(arrests.len(), 1);
    let first = arrests[0];
    assert_eq!(
        fixture
            .state
            .legal()
            .get_arrest(first)
            .expect("autonomous arrest should persist")
            .evidence(),
        &BTreeSet::from([fixture.evidence, corroborating])
    );

    validate_release_arrest(&fixture.state, first)
        .expect("autonomous custody should remain canonically releasable")
        .commit(&mut fixture.state)
        .expect("release should commit");
    assert!(
        apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
            .expect("post-release autonomous custody pass should resolve")
            .is_empty(),
        "unchanged evidence must not create an automatic release/re-arrest loop"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_for_investigation(fixture.investigation)
            .count(),
        1
    );

    // Explicit legal action may deliberately re-arrest after release, but it must satisfy the
    // same evidentiary custody threshold as autonomous policing.
    let explicit_rearrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence, corroborating]),
        },
    )
    .expect("the canonical command may deliberately re-arrest after release")
    .commit(&mut fixture.state)
    .expect("explicit re-arrest should commit");
    assert_ne!(explicit_rearrest, first);
    validate_state(&fixture.state).expect("one-shot autonomous custody state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn arrest_validation_is_case_specific_and_stales_when_case_evidence_changes() {
    let mut fixture = fixture();
    let corroborating = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let stale = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence, corroborating]),
        },
    )
    .expect("initial arrest plan should validate");
    add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let error = stale
        .commit(&mut fixture.state)
        .expect_err("case mutation must stale a previously validated arrest");
    assert!(matches!(error, ArrestError::StaleInvestigation { .. }));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );

    let second_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Separate case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
        },
    )
    .expect("second investigation should validate")
    .commit(&mut fixture.state)
    .expect("second investigation should commit");
    let foreign_evidence = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        second_case,
        fixture.suspect,
    );
    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([foreign_evidence]),
        },
    )
    .expect_err("evidence from another case must not support this arrest");
    assert_eq!(
        error,
        ArrestError::EvidenceInvestigationMismatch {
            evidence: foreign_evidence,
            investigation: fixture.investigation,
        }
    );
    validate_state(&fixture.state).expect("rejected arrest attempts must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn active_detention_blocks_case_suspension_and_membership_escape_until_release() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);

    // Suspension stays blocked while an arrest holds someone in custody; closing remains
    // allowed because a case whose subject is detained is cleared by arrest.
    let transition_error = validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .expect_err("active detention must keep its source case unsuspended");
    assert_eq!(
        transition_error,
        InvestigationError::ActiveArrestBlocksTransition {
            investigation: fixture.investigation,
            arrest,
        }
    );
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Close,
    )
    .expect("a cleared case must close while its subject is in custody");
    let reassignment_error =
        validate_reassign_character(&fixture.state, fixture.suspect, None, None)
            .expect_err("detained character must not escape custody through reassignment");
    assert_eq!(
        reassignment_error,
        WorldError::ActiveArrestAssignment {
            character: fixture.suspect,
            arrest,
        }
    );

    validate_release_arrest(&fixture.state, arrest)
        .expect("detention should release")
        .commit(&mut fixture.state)
        .expect("release should commit");
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Close,
    )
    .expect("released custody no longer requires an active source case")
    .commit(&mut fixture.state)
    .expect("case close should commit after release");
    validate_reassign_character(&fixture.state, fixture.suspect, None, None)
        .expect("released character should permit ordinary membership changes")
        .commit(&mut fixture.state)
        .expect("membership change should commit after release");
    validate_state(&fixture.state).expect("post-release lifecycle state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn detention_preserves_formal_supervision_but_blocks_new_supervisory_work() {
    let mut fixture = fixture();
    let crew = fixture
        .state
        .world()
        .get_character(fixture.suspect)
        .and_then(|record| record.organization())
        .expect("suspect fixture should belong to the criminal organization");
    let direct_report = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Existing Direct Report".to_owned(),
            organization: Some(crew),
            supervisor: Some(fixture.suspect),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("preexisting reporting line should validate");
    let unassigned = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unassigned Member".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unassigned member should validate");

    let arrest = arrest_fixture(&mut fixture);
    assert_eq!(
        fixture
            .state
            .world()
            .direct_reports(fixture.suspect)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![direct_report]
    );
    validate_state(&fixture.state)
        .expect("formal reporting lines may persist while a supervisor is detained");
    validate_invariants(&fixture.state);

    let error = validate_reassign_character(
        &fixture.state,
        unassigned,
        Some(crew),
        Some(fixture.suspect),
    )
    .expect_err("detained supervisor must not receive new reporting responsibility");
    assert_eq!(
        error,
        WorldError::DetainedSupervisor {
            supervisor: fixture.suspect,
            arrest,
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(unassigned)
            .expect("rejected reassignment must retain the character")
            .supervisor(),
        None
    );
    validate_state(&fixture.state).expect("rejected supervisory work must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn custody_defers_authorized_operation_until_participant_release() {
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{OperationApproach, OperationDraft, OperationKind, OperationObjective};
    use crate::world::world_system::{insert_business, insert_neighborhood};
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, NeighborhoodDraft,
        NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
    };

    let mut fixture = fixture();
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Arrest Guard Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Arrest Guard Front".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business should validate");
    let operation = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Guarded score".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: fixture
                .state
                .world()
                .get_character(fixture.suspect)
                .and_then(|record| record.organization())
                .expect("suspect should hold membership"),
            leader: fixture.suspect,
            objective: OperationObjective::ObtainCash {
                target: crate::core::entity::EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(crate::operations::RoleKind::Coordinator, fixture.suspect)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: crate::core::time::SimTime::ZERO,
        },
    )
    .expect("authorized operation should validate")
    .commit(&mut fixture.state)
    .expect("authorized operation should commit");

    let corroborating = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence, corroborating]),
        },
    )
    .expect("custody should validate despite the future operation booking")
    .commit(&mut fixture.state)
    .expect("custody should preserve an operation that has not started");
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_some_and(|record| record.id() == arrest),
        "arrest should become the authoritative live commitment"
    );
    let operation_record = fixture
        .state
        .operations()
        .get_operation(operation)
        .expect("authorized operation should persist");
    assert_eq!(
        operation_record.status(),
        crate::operations::OperationStatus::Authorized
    );
    validate_state(&fixture.state)
        .expect("detention may coexist with an authorized operation that has not started");

    let blocked_tick = crate::core::simulation::run_tick(&fixture.registry, &mut fixture.state);
    assert!(blocked_tick.started_operations.is_empty());
    assert_eq!(
        fixture
            .state
            .operations()
            .get_operation(operation)
            .expect("deferred operation should persist")
            .status(),
        crate::operations::OperationStatus::Authorized,
        "detention is temporary unavailability, not a before-start operation failure"
    );

    let maximum_detention = fixture.registry.legal().maximum_detention();
    fixture.state.advance_clock(SimDuration::from_minutes(
        maximum_detention.as_minutes() - 2,
    ));
    let released_tick = crate::core::simulation::run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(released_tick.custody_releases, vec![arrest]);
    assert_eq!(released_tick.started_operations, vec![operation]);
    assert_eq!(
        fixture
            .state
            .operations()
            .get_operation(operation)
            .expect("released operation should persist")
            .status(),
        crate::operations::OperationStatus::InProgress
    );
    validate_state(&fixture.state).expect("deferred custody operation state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn derived_forensic_evidence_cannot_satisfy_the_custody_bar_alone() {
    use crate::core::entity::EntityRef;
    use crate::core::simulation::run_tick;
    use crate::legal::investigation_system::{
        validate_assign_investigator, validate_incident_intake,
    };
    use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
    use crate::legal::{
        IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationWorkDraft, InvestigationWorkFocus,
        InvestigationWorkKind,
    };
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{OperationApproach, OperationDraft, OperationKind, OperationObjective};
    use crate::world::world_system::{insert_business, insert_character, insert_neighborhood};
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, Rating,
    };

    let mut fixture = fixture();
    let crew = fixture
        .state
        .world()
        .get_character(fixture.suspect)
        .and_then(|record| record.organization())
        .expect("suspect should hold membership");
    let leader = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unbooked Crew Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let detective = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Forensic Detective".to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(90).expect("fixture rating should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Corroboration Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Corroboration Front".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business should validate");
    // The suspect holds no operation booking: a different member leads the score.
    let operation = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Someone else's score".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(crate::operations::RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: crate::core::time::SimTime::ZERO,
        },
    )
    .expect("operation should authorize")
    .commit(&mut fixture.state)
    .expect("operation should commit");
    // An operation-originated case with exactly ONE independent strong item on the suspect.
    let outcome = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: fixture.police,
            title: "Single-source inquiry".to_owned(),
            subjects: BTreeSet::from([
                EntityRef::Operation(operation),
                EntityRef::Character(fixture.suspect),
            ]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Character(fixture.suspect),
                origin: Some(EntityRef::Operation(operation)),
                kind: EvidenceKind::Fingerprint,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(operation)),
            notified_organizations: BTreeSet::from([crew]),
            witness: None,
        },
    )
    .expect("originated case should validate")
    .commit(&mut fixture.state)
    .expect("originated case should commit");
    let case = outcome.investigation;
    let source = *outcome
        .evidence
        .first()
        .expect("intake carries its evidence");
    validate_assign_investigator(&fixture.state, case, detective)
        .expect("case staffing should validate")
        .commit(&mut fixture.state)
        .expect("case staffing should commit");

    // Develop the source: the forensic derivative clones its subject and strength.
    let work = validate_schedule_investigation_work(
        &fixture.registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: case,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(source),
        },
    )
    .expect("reviewable source should schedule")
    .commit(&mut fixture.state)
    .expect("review should commit");
    loop {
        let outcome = run_tick(&fixture.registry, &mut fixture.state);
        if outcome.resolved_investigation_work.contains(&work) {
            break;
        }
    }
    let derived = fixture
        .state
        .legal()
        .work_for_investigation(case)
        .find_map(|entry| {
            entry
                .resolution()
                .and_then(|resolution| resolution.derived_evidence())
        })
        .expect("the review must have produced a forensic derivative");

    let error = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: case,
            evidence: BTreeSet::from([source, derived]),
        },
    )
    .expect_err("a source and its forensic derivative must remain one corroborating fact");
    assert_eq!(
        error,
        ArrestError::InsufficientIndependentEvidence {
            found: 1,
            required: fixture
                .registry
                .legal()
                .minimum_arrest_qualifying_evidence(),
        }
    );

    // One independent item plus its own derivative is still one fact: no custody.
    assert!(
        apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
            .expect("autonomous arrest pass should resolve")
            .is_empty(),
        "a derived analysis must not corroborate its own source"
    );

    // A second INDEPENDENT strong item completes the corroboration bar and custody follows.
    let second = add_character_evidence(&mut fixture.state, fixture.police, case, fixture.suspect);
    validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: case,
            evidence: BTreeSet::from([derived, second]),
        },
    )
    .expect("a forensic derivative may stand in for its source without creating a new source");
    let arrests = apply_autonomous_evidence_arrests(&fixture.registry, &mut fixture.state)
        .expect("autonomous arrest pass should resolve");
    assert_eq!(arrests.len(), 1);
    let record = fixture
        .state
        .legal()
        .get_arrest(arrests[0])
        .expect("arrest record should persist");
    assert_eq!(record.character(), fixture.suspect);
    assert_eq!(
        record.evidence(),
        &BTreeSet::from([source, second]),
        "autonomous custody cites the primary evidence sources"
    );
    validate_state(&fixture.state).expect("custody state should remain valid");
    validate_invariants(&fixture.state);
}
