//! Focused tests for witness registration, interviews, testimony, and pressure effects.

use super::*;
use crate::build_registry;
use crate::core::id::{IdExhaustionError, IdKind};
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::time::{SimDuration, SimTime};
use crate::intelligence::{InformationSignal, KnowledgeHolder, LegalPersonStatusSignal};
use crate::legal::investigation_system::{
    InvestigationError, InvestigationTransition, apply_autonomous_investigator_staffing,
    validate_add_evidence, validate_assign_investigator, validate_open_investigation,
    validate_transition_investigation,
};
use crate::legal::{
    CaseWitnessRecord, EvidenceDraft, EvidenceRecord, InvestigationDraft, InvestigationRecord,
    WitnessStatementDraft,
};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct WitnessFixture {
    state: AppState,
    police: crate::core::id::OrganizationId,
    criminal: crate::core::id::OrganizationId,
    investigation: InvestigationId,
    witness: CharacterId,
    subject: CharacterId,
}

#[derive(Clone, Serialize)]
struct CaseWitnessRecordWire {
    id: CaseWitnessId,
    investigation: InvestigationId,
    witness: CharacterId,
    subject: EntityRef,
    cooperation: WitnessCooperation,
    registered_at: SimTime,
    statements: BTreeSet<crate::core::id::WitnessStatementId>,
    interview_attempts: u8,
    version: u32,
}

#[derive(Clone, Serialize)]
struct EvidenceIdentityWire {
    id: EvidenceId,
    investigation: InvestigationId,
    custodian: crate::core::id::OrganizationId,
}

#[derive(Clone, Serialize)]
struct EvidenceConnectionWire {
    subject: EntityRef,
    origin: Option<EntityRef>,
    source: Option<EntityRef>,
    derived_from: BTreeSet<EvidenceId>,
}

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
struct InvestigationRecordWire {
    id: InvestigationId,
    owner: crate::core::id::OrganizationId,
    title: String,
    status: InvestigationStatus,
    lead_investigator: Option<CharacterId>,
    declared_subjects: BTreeSet<EntityRef>,
    subjects: BTreeSet<EntityRef>,
    evidence: BTreeSet<EvidenceId>,
    opened_at: SimTime,
    origin: Option<EntityRef>,
    last_activity_at: SimTime,
    version: u32,
}

fn investigation_wire(record: &InvestigationRecord) -> InvestigationRecordWire {
    InvestigationRecordWire {
        id: record.id(),
        owner: record.owner(),
        title: record.title().to_owned(),
        status: record.status(),
        lead_investigator: record.lead_investigator(),
        declared_subjects: record.declared_subjects().clone(),
        subjects: record.subjects().clone(),
        evidence: record.evidence().clone(),
        opened_at: record.opened_at(),
        origin: record.origin(),
        last_activity_at: record.last_activity_at(),
        version: record.version(),
    }
}

fn replace_serialized_investigation(
    envelope: SaveEnvelope,
    original: &InvestigationRecord,
    replacement: &InvestigationRecordWire,
) -> SaveEnvelope {
    let original_bytes =
        bincode::serialize(original).expect("investigation record should serialize");
    let mirror = investigation_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("investigation mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement investigation should serialize");
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
        "serialized investigation must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout investigation corruption must remain decodable")
}

fn evidence_wire(record: &EvidenceRecord) -> EvidenceRecordWire {
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
    original: &EvidenceRecord,
    replacement: &EvidenceRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("evidence should serialize");
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
        "serialized evidence must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout evidence corruption must remain decodable")
}

fn case_witness_wire(record: &CaseWitnessRecord) -> CaseWitnessRecordWire {
    CaseWitnessRecordWire {
        id: record.id(),
        investigation: record.investigation(),
        witness: record.witness(),
        subject: record.subject(),
        cooperation: record.cooperation(),
        registered_at: record.registered_at(),
        statements: record.statements().clone(),
        interview_attempts: record.interview_attempts(),
        version: record.version(),
    }
}

fn replace_serialized_case_witness(
    envelope: SaveEnvelope,
    original: &CaseWitnessRecord,
    replacement: &CaseWitnessRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("case witness should serialize");
    let mirror = case_witness_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("case witness mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement case witness should serialize");
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
        "serialized case witness must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout case witness corruption must remain decodable")
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn make_fixture() -> WitnessFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x7117_E551);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Daniel Mercer".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness fixture should validate");
    let subject = insert_character(
        &mut state,
        CharacterDraft {
            name: "Frank Dello".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("subject fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Witness identification inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(subject)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("investigation fixture should commit");
    WitnessFixture {
        state,
        police,
        criminal,
        investigation,
        witness,
        subject,
    }
}

fn insert_detective(fixture: &mut WitnessFixture, name: &str) -> CharacterId {
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: name.to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(
                crate::world::CapabilityKind::Investigation,
                rating(70),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective fixture should validate")
}

fn assert_lead_knows_witness(fixture: &WitnessFixture, lead: CharacterId) {
    let records: Vec<_> = fixture
        .state
        .intelligence()
        .information_for_holder_subject(
            KnowledgeHolder::Character(lead),
            EntityRef::Character(fixture.witness),
        )
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].signal(),
        Some(&InformationSignal::LegalPersonStatus(
            LegalPersonStatusSignal::CaseWitness {
                investigation: fixture.investigation,
            }
        ))
    );
    assert_eq!(records[0].observed_at(), fixture.state.now());
}

#[test]
fn witness_registration_informs_an_existing_case_lead() {
    let mut fixture = make_fixture();
    let detective = insert_detective(&mut fixture, "Existing Witness Lead");
    validate_assign_investigator(&fixture.state, fixture.investigation, detective)
        .expect("detective should be assignable")
        .commit(&mut fixture.state)
        .expect("detective assignment should commit");

    validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("witness should register on a staffed case")
    .commit(&mut fixture.state)
    .expect("staffed witness registration should commit atomically");

    assert_lead_knows_witness(&fixture, detective);
    validate_state(&fixture.state).expect("staffed witness knowledge should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn later_case_staffing_learns_witnesses_registered_before_the_lead() {
    let mut fixture = make_fixture();
    validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("unstaffed case should accept its witness")
    .commit(&mut fixture.state)
    .expect("unstaffed witness registration should commit");

    let detective = insert_detective(&mut fixture, "Later Witness Lead");
    validate_assign_investigator(&fixture.state, fixture.investigation, detective)
        .expect("later detective should be assignable")
        .commit(&mut fixture.state)
        .expect("staffing should materialize existing witness knowledge");

    assert_lead_knows_witness(&fixture, detective);
    validate_state(&fixture.state).expect("post-staffing witness knowledge should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn named_case_witness_cannot_become_lead_and_autonomous_staffing_skips_them() {
    let mut fixture = make_fixture();
    let conflicted = insert_detective(&mut fixture, "Witness Detective");
    let replacement = insert_detective(&mut fixture, "Independent Detective");
    let witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: conflicted,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("qualified detective may be a witness before taking a case role")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");

    assert_eq!(
        validate_assign_investigator(&fixture.state, fixture.investigation, conflicted)
            .expect_err("a named witness cannot lead the same investigation"),
        InvestigationError::InvestigatorIsCaseWitness {
            investigation: fixture.investigation,
            investigator: conflicted,
            witness,
        }
    );
    let staffed = apply_autonomous_investigator_staffing(&mut fixture.state)
        .expect("autonomous staffing should skip witness-conflicted detectives");
    assert_eq!(staffed, vec![(fixture.investigation, replacement)]);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist")
            .lead_investigator(),
        Some(replacement)
    );
    validate_state(&fixture.state).expect("witness-conflict staffing must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn current_case_lead_cannot_be_registered_as_named_witness() {
    let mut fixture = make_fixture();
    let detective = insert_detective(&mut fixture, "Lead Witness Conflict");
    validate_assign_investigator(&fixture.state, fixture.investigation, detective)
        .expect("detective should be assignable before witness conflict")
        .commit(&mut fixture.state)
        .expect("detective assignment should commit");

    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: detective,
                subject: EntityRef::Character(fixture.subject),
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect_err("the current lead cannot simultaneously become a factual witness"),
        WitnessError::WitnessIsLeadInvestigator {
            investigation: fixture.investigation,
            witness: detective,
        }
    );
    assert!(
        fixture
            .state
            .legal()
            .case_witness_for(fixture.investigation, detective)
            .is_none()
    );
    validate_state(&fixture.state).expect("rejected lead-witness conflict must preserve state");
    validate_invariants(&fixture.state);
}

#[test]
fn staffed_witness_registration_preflights_knowledge_allocation_before_mutation() {
    let mut fixture = make_fixture();
    let detective = insert_detective(&mut fixture, "Allocation Boundary Lead");
    validate_assign_investigator(&fixture.state, fixture.investigation, detective)
        .expect("detective should be assignable")
        .commit(&mut fixture.state)
        .expect("detective assignment should commit");

    let validated = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("staffed witness registration should validate before allocator exhaustion");
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Information, u32::MAX);
    let before = bincode::serialize(&fixture.state).expect("pre-commit state should serialize");
    let next_witness = fixture.state.ids.next_raw(IdKind::CaseWitness);

    assert_eq!(
        validated
            .commit(&mut fixture.state)
            .expect_err("information exhaustion must reject before witness mutation"),
        WitnessError::IdExhaustion(IdExhaustionError::Exhausted {
            kind: "information",
            next: u32::MAX,
        })
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected state should serialize"),
        before,
        "failed case-witness knowledge allocation must leave authoritative state untouched"
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::CaseWitness),
        next_witness,
        "witness allocator must not advance when the composite reservation fails"
    );
    assert!(
        fixture
            .state
            .legal()
            .case_witness_for(fixture.investigation, fixture.witness)
            .is_none()
    );
}

#[test]
fn case_subject_cannot_be_registered_as_case_witness() {
    let fixture = make_fixture();
    let next_witness = fixture.state.ids.next_raw(IdKind::CaseWitness);

    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.subject,
                subject: EntityRef::Character(fixture.subject),
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect_err("a case subject cannot simultaneously enter that case as its witness"),
        WitnessError::WitnessIsCaseSubject {
            investigation: fixture.investigation,
            witness: fixture.subject,
        }
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::CaseWitness),
        next_witness
    );
    assert!(
        fixture
            .state
            .legal()
            .case_witness_for(fixture.investigation, fixture.subject)
            .is_none()
    );
    validate_state(&fixture.state).expect("rejected self-conflicted witness state must stay valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_cannot_be_registered_for_subject_outside_case() {
    let mut fixture = make_fixture();
    let outsider = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unrelated Registration Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unrelated character fixture should validate");
    let next_witness = fixture.state.ids.next_raw(IdKind::CaseWitness);

    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                subject: EntityRef::Character(outsider),
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect_err("registration must bind testimony only to existing case subject matter"),
        WitnessError::WitnessSubjectOutsideCase {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(outsider),
        }
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::CaseWitness),
        next_witness,
        "rejected registration must not consume a case-witness id"
    );
    assert!(
        fixture
            .state
            .legal()
            .case_witness_for(fixture.investigation, fixture.witness)
            .is_none()
    );
    validate_state(&fixture.state)
        .expect("rejected unrelated witness subject must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_effective_investigation_subject_without_declared_or_evidence_provenance() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let outsider = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Forged Effective Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("outsider fixture should validate");
    let investigation = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation fixture should persist")
        .clone();
    assert_eq!(investigation.declared_subjects(), investigation.subjects());
    assert!(investigation.evidence().is_empty());
    let mut corrupted = investigation_wire(&investigation);
    corrupted.subjects = BTreeSet::from([EntityRef::Character(outsider)]);

    let error = restore_save(
        &registry,
        replace_serialized_investigation(
            build_save(&registry, &fixture.state)
                .expect("valid investigation should save before subject corruption"),
            &investigation,
            &corrupted,
        ),
    )
    .expect_err(
        "restore must derive effective investigation subjects from declared context and evidence",
    );
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidInvestigationDefinition {
                investigation: invalid
            }
        ) if invalid == fixture.investigation
    ));
}

#[test]
fn restore_rejects_statement_whose_only_case_connection_is_its_own_subject_promotion() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let lead = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Weakly Connected Lead".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("weak lead fixture should validate");
    let decoy = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unconnected Decoy".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("decoy fixture should validate");
    let weak_connection = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(lead),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("weak case connection should validate")
    .commit(&mut fixture.state)
    .expect("weak case connection should commit");
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(lead),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness should validate")
    .commit(&mut fixture.state)
    .expect("case witness should commit");
    let _statement = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(95),
            summary: "The witness develops the weak lead into an identified subject.".to_owned(),
        },
    )
    .expect("testimony may develop an already connected weak lead")
    .commit(&mut fixture.state)
    .expect("lead-developing testimony should commit");
    let investigation = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist");
    assert!(
        !investigation
            .declared_subjects()
            .contains(&EntityRef::Character(lead)),
        "evidence-derived subject promotion must not rewrite declared case provenance"
    );
    assert!(
        investigation
            .subjects()
            .contains(&EntityRef::Character(lead)),
        "actionable testimony should promote the connected lead into effective subjects"
    );
    let weak_record = fixture
        .state
        .legal()
        .get_evidence(weak_connection)
        .expect("weak connection should persist")
        .clone();
    let mut corrupted = evidence_wire(&weak_record);
    corrupted.connection.subject = EntityRef::Character(decoy);

    let error = restore_save(
        &registry,
        replace_serialized_evidence(
            build_save(&registry, &fixture.state)
                .expect("valid testimony should save before provenance corruption"),
            &weak_record,
            &corrupted,
        ),
    )
    .expect_err(
        "a promoted effective subject must not let testimony justify its own prior case relevance",
    );
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidCaseWitness {
                witness: invalid
            }
        ) if invalid == case_witness
    ));
}

#[test]
fn external_witness_cooperation_change_does_not_refresh_police_case_activity() {
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let activity_before = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .last_activity_at();

    fixture.state.advance_clock(SimDuration::from_minutes(60));
    validate_set_witness_cooperation(&fixture.state, case_witness, WitnessCooperation::Reluctant)
        .expect("external cooperation degradation should validate")
        .commit(&mut fixture.state)
        .expect("external cooperation degradation should commit");

    let investigation = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist after witness change");
    assert_eq!(
        investigation.last_activity_at(),
        activity_before,
        "a witness-side change must not reset the police institution's inactivity clock"
    );
    validate_state(&fixture.state).expect("witness-side cooperation change should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_witness_attempt_counter_without_completed_interview_history() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let record = fixture
        .state
        .legal()
        .get_case_witness(case_witness)
        .expect("registered witness should persist")
        .clone();
    assert_eq!(record.interview_attempts(), 0);
    assert_eq!(record.version(), 1);
    let mut corrupted = case_witness_wire(&record);
    corrupted.interview_attempts = 1;
    // Make the version superficially plausible as though one mutation occurred. Restore must
    // still reject because there is no completed WitnessInterview work record backing the
    // future-affecting attempt counter.
    corrupted.version = 2;

    let error = restore_save(
        &registry,
        replace_serialized_case_witness(
            build_save(&registry, &fixture.state)
                .expect("valid case witness should save before attempt corruption"),
            &record,
            &corrupted,
        ),
    )
    .expect_err("interview attempts must be derived from completed interview work");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidCaseWitness {
                witness: invalid
            }
        ) if invalid == case_witness
    ));
}

#[test]
fn named_witness_statement_creates_source_bearing_testimony_and_survives_save() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let outcome = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: Some(EntityRef::Organization(fixture.criminal)),
            confidence: rating(88),
            summary: "Mercer identifies Frank Dello as the man he saw leaving the crew's garage."
                .to_owned(),
        },
    )
    .expect("named witness statement should validate")
    .commit(&mut fixture.state)
    .expect("named witness statement should commit");

    let statement = fixture
        .state
        .legal()
        .get_witness_statement(outcome.statement)
        .expect("statement should exist");
    assert_eq!(statement.case_witness(), case_witness);
    assert_eq!(statement.evidence(), outcome.evidence);
    assert_eq!(statement.confidence(), rating(88));
    let evidence = fixture
        .state
        .legal()
        .get_evidence(outcome.evidence)
        .expect("statement evidence should exist");
    assert_eq!(evidence.kind(), EvidenceKind::WitnessTestimony);
    assert_eq!(evidence.strength(), EvidenceStrength::Direct);
    assert_eq!(evidence.reliability(), EvidenceReliability::HighlyReliable);
    assert_eq!(evidence.admissibility(), Admissibility::Unknown);
    assert_eq!(evidence.subject(), EntityRef::Character(fixture.subject));
    assert_eq!(
        evidence.origin(),
        Some(EntityRef::Organization(fixture.criminal))
    );
    assert_eq!(
        evidence.source(),
        Some(EntityRef::Character(fixture.witness))
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_evidence(outcome.evidence)
            .map(|record| record.source()),
        Some(Some(EntityRef::Character(fixture.witness)))
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .witness_statement_for_evidence(outcome.evidence)
            .map(|record| record.id()),
        Some(outcome.statement)
    );
    assert_eq!(
        validate_record_witness_statement(
            &registry,
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                origin: None,
                confidence: rating(95),
                summary: "Mercer repeats the same identification.".to_owned(),
            },
        )
        .expect_err("one case witness cannot manufacture corroboration by repeating testimony"),
        WitnessError::WitnessAlreadyStatemented(case_witness)
    );

    let mut restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("named testimony state should save"),
    )
    .expect("named testimony state should restore");
    let restored_evidence = restored
        .legal()
        .get_evidence(outcome.evidence)
        .expect("restored witness evidence should exist");
    assert_eq!(
        restored_evidence.source(),
        Some(EntityRef::Character(fixture.witness))
    );
    assert_eq!(
        restored
            .legal()
            .witness_statement_for_evidence(outcome.evidence)
            .map(|record| record.id()),
        Some(outcome.statement)
    );

    let second_witness = insert_character(
        &mut restored,
        CharacterDraft {
            name: "Nora Bell".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("post-restore witness fixture should validate");
    let second_case_witness = validate_register_case_witness(
        &restored,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: second_witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("post-restore witness registration should validate")
    .commit(&mut restored)
    .expect("post-restore witness registration should allocate a fresh ID");
    let second_statement = validate_record_witness_statement(
        &registry,
        &restored,
        WitnessStatementDraft {
            case_witness: second_case_witness,
            origin: None,
            confidence: rating(61),
            summary: "Bell separately places Dello near the garage that evening.".to_owned(),
        },
    )
    .expect("post-restore testimony should validate")
    .commit(&mut restored)
    .expect("post-restore testimony should allocate fresh statement and evidence IDs");
    assert!(second_case_witness.raw() > case_witness.raw());
    assert!(second_statement.statement.raw() > outcome.statement.raw());
    assert!(second_statement.evidence.raw() > outcome.evidence.raw());
    validate_state(&restored).expect("restored testimony state should be structurally valid");
    validate_state_against_registry(&registry, &restored)
        .expect("restored testimony state should remain registry-valid");
    validate_invariants(&restored);
}

#[test]
fn later_subject_promotion_preserves_historical_testimony_but_ends_witness_role() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("independent witness should register")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let historical = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(88),
            summary: "Mercer identifies Dello before later evidence implicates Mercer himself."
                .to_owned(),
        },
    )
    .expect("historical testimony should validate")
    .commit(&mut fixture.state)
    .expect("historical testimony should commit");

    validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.witness),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("later actionable evidence against the witness should validate")
    .commit(&mut fixture.state)
    .expect("later actionable evidence should commit");

    assert!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist")
            .subjects()
            .contains(&EntityRef::Character(fixture.witness)),
        "actionable evidence should promote the former witness into the arrest-eligible subject set"
    );
    assert!(
        fixture
            .state
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    assert!(
        fixture
            .state
            .legal()
            .get_evidence(historical.evidence)
            .is_some()
    );
    assert_eq!(
        validate_set_witness_cooperation(&fixture.state, case_witness, WitnessCooperation::Hostile)
            .expect_err("a current case subject cannot keep acting through witness cooperation"),
        WitnessError::WitnessIsCaseSubject {
            investigation: fixture.investigation,
            witness: fixture.witness,
        }
    );
    assert!(
        !crate::operations::operation_objective::has_pressureable_witness_case(
            &fixture.state,
            fixture.criminal,
            fixture.witness,
        ),
        "counter-play must not treat a newly arrest-eligible subject as a pressureable witness"
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("historical testimony plus later role conflict should remain save-valid"),
    )
    .expect("historical testimony plus later role conflict should restore");
    assert!(
        restored
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    assert!(restored.legal().get_evidence(historical.evidence).is_some());
    assert!(
        restored
            .legal()
            .get_investigation(fixture.investigation)
            .expect("restored investigation should persist")
            .subjects()
            .contains(&EntityRef::Character(fixture.witness))
    );
    validate_state(&restored)
        .expect("restored historical testimony should remain structurally valid");
    validate_state_against_registry(&registry, &restored)
        .expect("restored historical testimony should remain registry-valid");
    validate_invariants(&restored);
}

#[test]
fn witness_registration_and_cooperation_tokens_reject_case_and_statement_changes() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let stale_registration = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("registration should initially validate");
    validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("case mutation should validate")
    .commit(&mut fixture.state)
    .expect("case mutation should commit");
    assert!(matches!(
        stale_registration.commit(&mut fixture.state),
        Err(WitnessError::StaleInvestigation { .. })
    ));

    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("fresh registration should validate")
    .commit(&mut fixture.state)
    .expect("fresh registration should commit");
    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                subject: EntityRef::Character(fixture.subject),
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect_err("same character cannot be registered twice on one case"),
        WitnessError::DuplicateCaseWitness {
            investigation: fixture.investigation,
            witness: fixture.witness,
            existing: case_witness,
        }
    );

    let stale_cooperation = validate_set_witness_cooperation(
        &fixture.state,
        case_witness,
        WitnessCooperation::Cooperative,
    )
    .expect("cooperation change should initially validate");
    validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(55),
            summary: "Mercer says he is fairly sure Dello was present.".to_owned(),
        },
    )
    .expect("statement should validate")
    .commit(&mut fixture.state)
    .expect("statement should commit");
    assert!(matches!(
        stale_cooperation.commit(&mut fixture.state),
        Err(WitnessError::StaleCaseWitness { .. })
    ));
    validate_set_witness_cooperation(
        &fixture.state,
        case_witness,
        WitnessCooperation::Cooperative,
    )
    .expect("fresh cooperation token should validate")
    .commit(&mut fixture.state)
    .expect("fresh cooperation change should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_case_witness(case_witness)
            .expect("case witness should exist")
            .cooperation(),
        WitnessCooperation::Cooperative
    );
    validate_state(&fixture.state).expect("versioned witness state should remain valid");
}

#[test]
fn suspended_case_preserves_testimony_but_rejects_new_witness_activity() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.subject),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("registration should validate")
    .commit(&mut fixture.state)
    .expect("registration should commit");
    let historical = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(72),
            summary: "Mercer identifies Dello from the alley encounter.".to_owned(),
        },
    )
    .expect("historical statement should validate")
    .commit(&mut fixture.state)
    .expect("historical statement should commit");
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .expect("case suspension should validate")
    .commit(&mut fixture.state)
    .expect("case suspension should commit");

    let statement_error = match validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(90),
            summary: "Mercer offers a second identification.".to_owned(),
        },
    ) {
        Ok(_) => panic!("suspended case must reject new witness statements"),
        Err(error) => error,
    };
    assert_eq!(
        statement_error,
        WitnessError::InactiveInvestigation(fixture.investigation)
    );
    assert_eq!(
        validate_set_witness_cooperation(
            &fixture.state,
            case_witness,
            WitnessCooperation::Hostile,
        )
        .expect_err("suspended case must reject cooperation mutation"),
        WitnessError::InactiveInvestigation(fixture.investigation)
    );
    assert!(
        fixture
            .state
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    assert!(
        fixture
            .state
            .legal()
            .get_evidence(historical.evidence)
            .is_some()
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("suspended case with testimony should save"),
    )
    .expect("suspended case with testimony should restore");
    assert!(
        restored
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    validate_state(&restored).expect("historical testimony should survive suspension");
    validate_invariants(&restored);
}

#[test]
fn anonymous_witness_testimony_remains_valid_without_named_source() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.subject),
            origin: Some(EntityRef::Organization(fixture.criminal)),
            kind: EvidenceKind::WitnessTestimony,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("anonymous testimony should remain valid evidence")
    .commit(&mut fixture.state)
    .expect("anonymous testimony should commit");
    let record = fixture
        .state
        .legal()
        .get_evidence(evidence)
        .expect("anonymous testimony should exist");
    assert_eq!(record.kind(), EvidenceKind::WitnessTestimony);
    assert_eq!(record.source(), None);
    assert!(
        fixture
            .state
            .legal()
            .witness_statement_for_evidence(evidence)
            .is_none()
    );
    validate_state(&fixture.state).expect("anonymous testimony should remain structurally valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("anonymous testimony should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_confidence_maps_to_deterministic_evidence_bands() {
    let testimony = build_registry().legal().witness_testimony();
    for (confidence, strength) in [
        (0, EvidenceStrength::Weak),
        (34, EvidenceStrength::Weak),
        (35, EvidenceStrength::Corroborating),
        (59, EvidenceStrength::Corroborating),
        (60, EvidenceStrength::Strong),
        (84, EvidenceStrength::Strong),
        (85, EvidenceStrength::Direct),
        (100, EvidenceStrength::Direct),
    ] {
        assert_eq!(
            resolve_witness_strength(
                testimony,
                rating(confidence),
                WitnessCooperation::Cooperative,
            ),
            strength
        );
    }
    for (confidence, reliability) in [
        (0, EvidenceReliability::Questionable),
        (24, EvidenceReliability::Questionable),
        (25, EvidenceReliability::Mixed),
        (49, EvidenceReliability::Mixed),
        (50, EvidenceReliability::Credible),
        (79, EvidenceReliability::Credible),
        (80, EvidenceReliability::HighlyReliable),
        (100, EvidenceReliability::HighlyReliable),
    ] {
        assert_eq!(
            resolve_witness_reliability(
                testimony,
                rating(confidence),
                WitnessCooperation::Cooperative,
            ),
            reliability
        );
    }
}

#[test]
fn uncooperative_witnesses_cannot_produce_top_band_testimony() {
    let testimony = build_registry().legal().witness_testimony();
    for confidence in 0..=100 {
        let confidence = rating(confidence);
        for (cooperation, strength_cap, reliability_cap) in [
            (
                WitnessCooperation::Cooperative,
                EvidenceStrength::Direct,
                EvidenceReliability::HighlyReliable,
            ),
            (
                WitnessCooperation::Reluctant,
                EvidenceStrength::Strong,
                EvidenceReliability::Credible,
            ),
            (
                WitnessCooperation::Hostile,
                EvidenceStrength::Corroborating,
                EvidenceReliability::Mixed,
            ),
        ] {
            let strength = resolve_witness_strength(testimony, confidence, cooperation);
            assert!(strength <= strength_cap);
            let reliability = resolve_witness_reliability(testimony, confidence, cooperation);
            assert!(reliability <= reliability_cap);
        }
    }
}
