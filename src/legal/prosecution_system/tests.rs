//! Focused tests for prosecution referral, review, and resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::witness_system::{WitnessError, validate_register_case_witness};
use crate::legal::{
    Admissibility, ArrestDraft, CaseWitnessDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, InvestigationDraft, WitnessCooperation,
};
use crate::registry::Registry;
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    police: OrganizationId,
    office: OrganizationId,
    defendant: CharacterId,
    lead: CharacterId,
    investigation: InvestigationId,
    arrest: ArrestId,
    arrest_evidence: EvidenceId,
    arrest_corroboration: EvidenceId,
    supplemental_evidence: EvidenceId,
}

fn arrest_evidence_set(fixture: &Fixture) -> BTreeSet<EvidenceId> {
    BTreeSet::from([fixture.arrest_evidence, fixture.arrest_corroboration])
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
        "serialized evidence must appear exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout evidence corruption must remain decodable")
}

#[derive(Clone, Serialize)]
struct ArrestRecordWire {
    id: ArrestId,
    character: CharacterId,
    authority: OrganizationId,
    investigation: InvestigationId,
    evidence: BTreeSet<EvidenceId>,
    arrested_at: SimTime,
    released_at: Option<SimTime>,
    status: crate::legal::ArrestStatus,
    version: u32,
}

#[test]
fn prosecution_referrals_reject_evidence_about_another_person_in_the_same_police_case() {
    let mut fixture = fixture();
    let unrelated_character = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unrelated Case Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unrelated character fixture should validate");
    let unrelated = add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        unrelated_character,
        EvidenceKind::Surveillance,
    );
    let opening_error = match validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence: BTreeSet::from([
                fixture.arrest_evidence,
                fixture.arrest_corroboration,
                unrelated,
            ]),
        },
    ) {
        Ok(_) => panic!("initial referral must not import unrelated evidence from the police file"),
        Err(error) => error,
    };
    assert_eq!(
        opening_error,
        ProsecutionError::EvidenceDefendantMismatch {
            evidence: unrelated,
            defendant: fixture.defendant,
        }
    );

    let case = open_case(&mut fixture);
    let supplement_error = match validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([unrelated]),
        },
    ) {
        Ok(_) => panic!("supplement must remain defendant-specific"),
        Err(error) => error,
    };
    assert_eq!(
        supplement_error,
        ProsecutionError::EvidenceDefendantMismatch {
            evidence: unrelated,
            defendant: fixture.defendant,
        }
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("case should remain unchanged")
            .evidence(),
        &arrest_evidence_set(&fixture)
    );
    validate_state(&fixture.state).expect("rejected unrelated referrals leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_referred_evidence_that_no_longer_concerns_the_defendant() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("valid defendant-specific supplement should validate")
    .commit(&mut fixture.state)
    .expect("valid defendant-specific supplement should commit");

    let original = fixture
        .state
        .legal()
        .get_evidence(fixture.supplemental_evidence)
        .expect("supplemental evidence should persist");
    let mut corrupted = evidence_wire(original);
    corrupted.connection.subject = EntityRef::Character(fixture.lead);
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid prosecution state should save before corruption");
    let corrupted = replace_serialized_evidence(envelope, original, &corrupted);

    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("restore must reject a referral whose evidence was retargeted off defendant");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidProsecutionReferral { .. }
        )
    ));
}

fn arrest_wire(record: &crate::legal::ArrestRecord) -> ArrestRecordWire {
    ArrestRecordWire {
        id: record.id(),
        character: record.character(),
        authority: record.authority(),
        investigation: record.investigation(),
        evidence: record.evidence().clone(),
        arrested_at: record.arrested_at(),
        released_at: record.released_at(),
        status: record.status(),
        version: record.version(),
    }
}

fn replace_serialized_arrest(
    envelope: SaveEnvelope,
    original: &crate::legal::ArrestRecord,
    replacement: &ArrestRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("arrest should serialize");
    let mirror = arrest_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("arrest mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement arrest should serialize");
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
        "serialized arrest must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout arrest corruption must remain decodable")
}

fn tamper_serialized_summary(
    envelope: SaveEnvelope,
    summary: &str,
    expected_occurrences: usize,
) -> SaveEnvelope {
    assert!(!summary.is_empty());
    assert!(
        summary.is_ascii(),
        "fixture summaries must be byte-stable ASCII"
    );
    let needle = summary.as_bytes();
    let mut bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let mut cursor = 0;
    let mut replaced = 0;
    while cursor + needle.len() <= bytes.len() {
        let Some(relative) = bytes[cursor..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            break;
        };
        let start = cursor + relative;
        bytes[start..start + needle.len()].fill(b'X');
        cursor = start + needle.len();
        replaced += 1;
    }
    assert_eq!(
        replaced, expected_occurrences,
        "test must corrupt exactly the intended persisted summary copies"
    );
    bincode::deserialize(&bytes).expect("equal-length summary corruption must remain decodable")
}

#[test]
fn restore_rejects_prosecution_predating_its_arrest_anchor() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let opened_at = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("prosecution case should persist")
        .opened_at();
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::ONE_MINUTE);
    let arrest = fixture
        .state
        .legal()
        .get_arrest(fixture.arrest)
        .expect("source arrest should persist")
        .clone();
    assert_eq!(arrest.arrested_at(), opened_at);
    let mut corrupted = arrest_wire(&arrest);
    corrupted.arrested_at = fixture.state.now();

    let error = restore_save(
        &fixture.registry,
        replace_serialized_arrest(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid prosecution state should save before chronology corruption"),
            &arrest,
            &corrupted,
        ),
    )
    .expect_err("prosecution cannot predate the arrest that anchors its case");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidProsecutionCase {
                case: invalid
            }
        ) if invalid == case
    ));
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn add_evidence(
    state: &mut AppState,
    police: OrganizationId,
    investigation: InvestigationId,
    defendant: CharacterId,
    kind: EvidenceKind,
) -> EvidenceId {
    validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(defendant),
            origin: None,
            kind,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("fixture evidence should validate")
    .commit(state)
    .expect("fixture evidence should commit")
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0xCA5E_1931);
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Canal Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Canal Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let office = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "District Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .expect("prosecutor office should validate");
    let defendant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Case Defendant".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("defendant fixture should validate");
    let lead = insert_character(
        &mut state,
        CharacterDraft {
            name: "Lead Prosecutor".to_owned(),
            organization: Some(office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(86))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("lead prosecutor fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Canal arrest case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(defendant)]),
        },
    )
    .expect("source investigation should validate")
    .commit(&mut state)
    .expect("source investigation should commit");
    let arrest_evidence = add_evidence(
        &mut state,
        police,
        investigation,
        defendant,
        EvidenceKind::Document,
    );
    let arrest_corroboration = add_evidence(
        &mut state,
        police,
        investigation,
        defendant,
        EvidenceKind::KnownAssociation,
    );
    let arrest = validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([arrest_evidence, arrest_corroboration]),
        },
    )
    .expect("arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");
    let supplemental_evidence = add_evidence(
        &mut state,
        police,
        investigation,
        defendant,
        EvidenceKind::FinancialRecord,
    );
    Fixture {
        registry,
        state,
        police,
        office,
        defendant,
        lead,
        investigation,
        arrest,
        arrest_evidence,
        arrest_corroboration,
        supplemental_evidence,
    }
}

fn opening_draft(fixture: &Fixture) -> ProsecutionCaseDraft {
    ProsecutionCaseDraft {
        arrest: fixture.arrest,
        prosecutor_office: fixture.office,
        prosecutor: fixture.lead,
        evidence: arrest_evidence_set(fixture),
    }
}

fn open_case(fixture: &mut Fixture) -> ProsecutionCaseId {
    validate_open_prosecution_case(&fixture.state, opening_draft(fixture))
        .expect("prosecution case should validate")
        .commit(&mut fixture.state)
        .expect("prosecution case should commit")
}

#[test]
fn prosecution_rejects_defendant_as_their_own_prosecutor() {
    let registry = build_registry();
    let mut state = AppState::new(0x5E1F_C45E);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Conflict Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let office = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Conflict Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .expect("prosecutor fixture should validate");
    let defendant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Prosecutor Defendant".to_owned(),
            organization: Some(office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("defendant fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Prosecutor self-conflict case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(defendant)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let first = add_evidence(
        &mut state,
        police,
        investigation,
        defendant,
        EvidenceKind::Document,
    );
    let second = add_evidence(
        &mut state,
        police,
        investigation,
        defendant,
        EvidenceKind::KnownAssociation,
    );
    let arrest = validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([first, second]),
        },
    )
    .expect("arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");

    assert_eq!(
        validate_open_prosecution_case(
            &state,
            ProsecutionCaseDraft {
                arrest,
                prosecutor_office: office,
                prosecutor: defendant,
                evidence: BTreeSet::from([first, second]),
            },
        )
        .err()
        .expect("a defendant must never prosecute their own case"),
        ProsecutionError::ProsecutorIsDefendant {
            prosecutor: defendant,
            defendant,
        }
    );
    validate_state(&state).expect("rejected self-prosecution must preserve valid state");
    validate_invariants(&state);
}

#[test]
fn source_case_subject_cannot_open_prosecution_as_prosecutor() {
    let mut fixture = fixture();
    add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.lead,
        EvidenceKind::Surveillance,
    );

    assert_eq!(
        validate_open_prosecution_case(&fixture.state, opening_draft(&fixture))
            .err()
            .expect("an actionable source-case subject cannot prosecute the same case"),
        ProsecutionError::ProsecutorIsCaseSubject {
            prosecutor: fixture.lead,
            investigation: fixture.investigation,
        }
    );
    validate_state(&fixture.state)
        .expect("rejected subject-prosecutor conflict must preserve canonical state");
    validate_invariants(&fixture.state);
}

#[test]
fn named_source_case_witness_cannot_open_prosecution_as_prosecutor() {
    let mut fixture = fixture();
    validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.lead,
            subject: EntityRef::Character(fixture.defendant),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("prosecutor may be a factual witness before taking a prosecution role")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");

    assert_eq!(
        validate_open_prosecution_case(&fixture.state, opening_draft(&fixture))
            .err()
            .expect("a source-case witness cannot prosecute that case"),
        ProsecutionError::ProsecutorIsCaseWitness {
            prosecutor: fixture.lead,
            investigation: fixture.investigation,
        }
    );
    validate_state(&fixture.state).expect("rejected witness-prosecutor conflict must stay valid");
    validate_invariants(&fixture.state);
}

#[test]
fn assigned_prosecutor_cannot_be_registered_as_source_case_witness() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);

    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.lead,
                subject: EntityRef::Character(fixture.defendant),
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect_err("an assigned prosecutor cannot become a factual witness in the source case"),
        WitnessError::WitnessIsAssignedProsecutor {
            investigation: fixture.investigation,
            witness: fixture.lead,
            case,
        }
    );
    validate_state(&fixture.state).expect("rejected prosecutor-witness conflict must stay valid");
    validate_invariants(&fixture.state);
}

#[test]
fn actionable_evidence_against_assigned_prosecutor_recuses_review_for_restaffing() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let backup = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Conflict-Free Backup Prosecutor".to_owned(),
            organization: Some(fixture.office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(70))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("backup prosecutor fixture should validate");

    let conflict_evidence = add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.lead,
        EvidenceKind::Surveillance,
    );
    assert!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("source investigation should persist")
            .subjects()
            .contains(&EntityRef::Character(fixture.lead)),
        "actionable evidence must promote the prosecutor into the source case subject set"
    );
    let reviewing = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("prosecution review should persist");
    assert_eq!(reviewing.assigned_prosecutor(), None);
    assert_eq!(reviewing.version(), 2);
    assert!(
        fixture
            .state
            .legal()
            .reviewing_prosecution_cases_for_prosecutor(fixture.lead)
            .all(|reviewing| reviewing.id() != case)
    );
    assert_eq!(
        validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        )
        .err()
        .expect("recused review cannot perform prosecution work until restaffed"),
        ProsecutionError::CaseUnstaffed { case }
    );

    let staffed = apply_autonomous_prosecution_staffing(&mut fixture.state)
        .expect("office should deterministically restaff after evidence-driven recusal");
    assert_eq!(staffed, vec![(case, backup)]);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("restaffed case should persist")
            .assigned_prosecutor(),
        Some(backup)
    );
    assert!(
        fixture
            .state
            .legal()
            .get_evidence(conflict_evidence)
            .is_some(),
        "the causative evidence remains authoritative after recusal"
    );
    validate_state(&fixture.state).expect("evidence-driven prosecutor recusal must stay valid");
    validate_invariants(&fixture.state);

    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("recused and restaffed prosecution should build a save"),
    )
    .expect("recused and restaffed prosecution should restore");
    assert_eq!(
        restored
            .legal()
            .get_prosecution_case(case)
            .expect("restored prosecution should persist")
            .assigned_prosecutor(),
        Some(backup)
    );
    validate_state(&restored).expect("restored prosecutor recusal state must remain valid");
    validate_invariants(&restored);
}

#[test]
fn former_prosecutor_may_become_source_case_witness_after_review_ends_in_same_minute() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    validate_decline_prosecution_case(&fixture.state, case)
        .expect("prosecution decline should validate")
        .commit(&mut fixture.state)
        .expect("prosecution decline should commit");

    let witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.lead,
            subject: EntityRef::Character(fixture.defendant),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("a former prosecutor may become a witness after the current review role ends")
    .commit(&mut fixture.state)
    .expect("post-review witness registration should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .case_witness_for(fixture.investigation, fixture.lead)
            .map(|record| record.id()),
        Some(witness)
    );
    validate_state(&fixture.state)
        .expect("same-minute post-review witness history must remain persistence-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn prosecution_staffing_and_autonomy_skip_source_case_witnesses() {
    let mut fixture = fixture();
    validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.lead,
            subject: EntityRef::Character(fixture.defendant),
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("lead prosecutor may be a witness before any prosecution assignment")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let opening_prosecutor = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Opening Prosecutor".to_owned(),
            organization: Some(fixture.office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(60))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("opening prosecutor fixture should validate");
    let backup = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Independent Backup Prosecutor".to_owned(),
            organization: Some(fixture.office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(70))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("backup prosecutor fixture should validate");
    let case = validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            prosecutor: opening_prosecutor,
            evidence: arrest_evidence_set(&fixture),
        },
    )
    .expect("non-witness prosecutor should open the case")
    .commit(&mut fixture.state)
    .expect("case opening should commit");

    fixture
        .state
        .legal
        .release_prosecution_case_prosecutor_for_detention(case, opening_prosecutor);
    assert_eq!(
        validate_assign_prosecutor(&fixture.state, case, fixture.lead)
            .expect_err("direct staffing must reject the source-case witness"),
        ProsecutionStaffingError::ProsecutorIsCaseWitness {
            case,
            prosecutor: fixture.lead,
        }
    );
    let staffed = apply_autonomous_prosecution_staffing(&mut fixture.state)
        .expect("autonomous staffing should skip witness-conflicted prosecutors");
    assert_eq!(staffed, vec![(case, backup)]);
    validate_state(&fixture.state).expect("restaffed witness-conflict state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_prosecution_staffing_prefers_lower_active_caseload_before_skill() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let backup = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Available Prosecutor".to_owned(),
            organization: Some(fixture.office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(70))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("backup prosecutor should validate");
    let record = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("staffed prosecution case should persist");

    assert_eq!(
        find_autonomous_prosecutor(&fixture.state, record),
        Some(backup),
        "an idle qualified prosecutor should be preferred over a stronger attorney already carrying review work"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn referral_preserves_police_custody_and_survives_save_before_supplement() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let record = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("prosecution case should persist");
    assert_eq!(record.status(), ProsecutionCaseStatus::Reviewing);
    assert_eq!(record.defendant(), fixture.defendant);
    assert_eq!(record.source_investigation(), fixture.investigation);
    assert_eq!(record.source_authority(), fixture.police);
    assert_eq!(record.prosecutor_office(), fixture.office);
    assert_eq!(record.assigned_prosecutor(), Some(fixture.lead));
    assert_eq!(record.evidence(), &arrest_evidence_set(&fixture));
    assert_eq!(record.version(), 1);
    let initial_referral = record.initial_referral();
    let referral = fixture
        .state
        .legal()
        .get_prosecution_referral(initial_referral)
        .expect("initial referral should persist");
    assert_eq!(referral.evidence(), record.evidence());
    assert_eq!(
        fixture
            .state
            .legal()
            .get_evidence(fixture.arrest_evidence)
            .expect("source evidence should persist")
            .custodian(),
        fixture.police
    );
    assert_eq!(
        fixture
            .state
            .intelligence()
            .get_information(referral.information())
            .expect("referral information should persist")
            .holder(),
        KnowledgeHolder::Organization(fixture.office)
    );
    validate_state(&fixture.state).expect("initial prosecution referral should validate");
    validate_invariants(&fixture.state);

    let save = build_save(&fixture.registry, &fixture.state)
        .expect("prosecution referral should build a save");
    let bytes = bincode::serialize(&save).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored =
        restore_save(&fixture.registry, decoded).expect("prosecution referral should restore");
    let supplemental = validate_supplement_prosecution_case(
        &restored,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("supplemental referral should validate after restore")
    .commit(&mut restored)
    .expect("supplemental referral should commit after restore");
    assert_ne!(supplemental, initial_referral);
    let updated = restored
        .legal()
        .get_prosecution_case(case)
        .expect("supplemented prosecution case should persist");
    assert_eq!(updated.version(), 2);
    assert_eq!(updated.referrals().len(), 2);
    assert_eq!(
        updated.evidence(),
        &BTreeSet::from([
            fixture.arrest_evidence,
            fixture.arrest_corroboration,
            fixture.supplemental_evidence,
        ])
    );
    assert_eq!(
        restored
            .legal()
            .get_evidence(fixture.supplemental_evidence)
            .expect("supplemental source evidence should persist")
            .custodian(),
        fixture.police
    );
    validate_state(&restored).expect("supplemented restored prosecution case should validate");
    validate_invariants(&restored);
}

#[test]
fn prosecution_referral_rejects_matching_but_unauthored_persisted_summary() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let (referral_id, summary) = {
        let case_record = fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist");
        let referral_id = case_record.initial_referral();
        let referral = fixture
            .state
            .legal()
            .get_prosecution_referral(referral_id)
            .expect("initial referral should persist");
        let summary = fixture
            .state
            .intelligence()
            .get_information(referral.information())
            .expect("referral information should persist")
            .summary()
            .to_owned();
        (referral_id, summary)
    };
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid prosecution referral should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 2);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten referral narrative must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidProsecutionReferral {
                    referral: invalid,
                }
            ) if invalid == referral_id
        ),
        "expected invalid prosecution referral, got {error:?}"
    );
}

#[test]
fn initial_referral_must_include_every_evidence_record_that_supported_arrest() {
    let fixture = fixture();
    let error = match validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            prosecutor: fixture.lead,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    ) {
        Ok(_) => panic!("prosecution intake must not omit arrest evidence"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        ProsecutionError::MissingArrestEvidence(fixture.arrest_evidence)
    );
    assert!(
        fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none()
    );
    validate_state(&fixture.state).expect("rejected referral should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn supplemental_referral_stales_when_source_police_case_changes() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let stale = validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("supplement should initially validate");
    add_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.defendant,
        EvidenceKind::Surveillance,
    );
    let error = stale
        .commit(&mut fixture.state)
        .expect_err("source case mutation must stale supplemental referral");
    assert!(matches!(error, ProsecutionError::StaleInvestigation { .. }));
    let record = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("prosecution case should remain");
    assert_eq!(record.version(), 1);
    assert!(!record.evidence().contains(&fixture.supplemental_evidence));
    assert_eq!(record.referrals().len(), 1);
    validate_state(&fixture.state).expect("stale referral rejection should be atomic");
    validate_invariants(&fixture.state);
}

#[test]
fn open_case_is_unique_per_office_but_other_prosecutor_office_may_receive_referral() {
    let mut fixture = fixture();
    let first = open_case(&mut fixture);
    let duplicate = match validate_open_prosecution_case(&fixture.state, opening_draft(&fixture)) {
        Ok(_) => panic!("same office must not open duplicate case for one arrest"),
        Err(error) => error,
    };
    assert_eq!(
        duplicate,
        ProsecutionError::DuplicateOpenCase {
            arrest: fixture.arrest,
            office: fixture.office,
            case: first,
        }
    );

    let second_office = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "State Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .expect("second prosecutor office should validate");
    let second_lead = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "State Prosecutor Lead".to_owned(),
            organization: Some(second_office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(91))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second prosecutor should validate");
    let second = validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: second_office,
            prosecutor: second_lead,
            evidence: arrest_evidence_set(&fixture),
        },
    )
    .expect("different prosecutor office may receive same arrest referral")
    .commit(&mut fixture.state)
    .expect("second office case should commit");
    assert_ne!(first, second);
    assert_eq!(
        fixture
            .state
            .legal()
            .prosecution_cases()
            .filter(|record| record.arrest() == fixture.arrest)
            .count(),
        2
    );
    validate_decline_prosecution_case(&fixture.state, first)
        .expect("one office may end review while another remains open")
        .commit(&mut fixture.state)
        .expect("first office decline should commit");
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.defendant)
            .is_some_and(|arrest| arrest.id() == fixture.arrest),
        "another office's live prosecution review must keep the originating arrest detained"
    );
    validate_decline_prosecution_case(&fixture.state, second)
        .expect("last office may end review")
        .commit(&mut fixture.state)
        .expect("last office decline should commit");
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.defendant)
            .is_none(),
        "ending the final prosecution review must release the originating detention"
    );
    validate_state(&fixture.state).expect("multiple-office referral state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn open_prosecution_case_blocks_lead_transfer_but_not_formal_case_persistence() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let error = validate_reassign_character(&fixture.state, fixture.lead, None, None)
        .expect_err("open prosecution assignment must block office transfer");
    assert_eq!(
        error,
        WorldError::ActiveProsecutionAssignment {
            character: fixture.lead,
            case,
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.lead)
            .expect("lead should persist")
            .organization(),
        Some(fixture.office)
    );
    validate_state(&fixture.state).expect("rejected lead transfer should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn declining_case_releases_lead_assignment_and_ends_referral_access() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(15));

    validate_decline_prosecution_case(&fixture.state, case)
        .expect("reviewing case should be eligible for decline")
        .commit(&mut fixture.state)
        .expect("decline should commit atomically");
    let record = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("declined prosecution case should persist");
    assert_eq!(record.status(), ProsecutionCaseStatus::Declined);
    assert_eq!(record.resolved_at(), Some(fixture.state.now()));
    assert!(record.resolution_information().is_some());
    assert!(record.resolution_report().is_some());
    assert_eq!(record.version(), 2);
    assert!(
        fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none()
    );
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.defendant)
            .is_none()
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_arrest(fixture.arrest)
            .expect("originating arrest should persist as history")
            .status(),
        crate::legal::ArrestStatus::Released
    );

    let supplement_error = match validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    ) {
        Ok(_) => panic!("declined case must reject later evidence referral"),
        Err(error) => error,
    };
    assert_eq!(supplement_error, ProsecutionError::CaseNotOpen { case });

    validate_reassign_character(&fixture.state, fixture.lead, None, None)
        .expect("terminal prosecution case must release lead organization lock")
        .commit(&mut fixture.state)
        .expect("released lead should be able to leave prosecutor office");
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.lead)
            .expect("lead prosecutor should persist")
            .organization(),
        None
    );
    validate_state(&fixture.state).expect("declined historical case should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn closed_case_survives_save_and_allows_later_reconsideration() {
    let mut fixture = fixture();
    let first = open_case(&mut fixture);
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(30));
    validate_close_prosecution_case(&fixture.state, first)
        .expect("reviewing case should be eligible for closure")
        .commit(&mut fixture.state)
        .expect("case closure should commit");

    let save = build_save(&fixture.registry, &fixture.state)
        .expect("closed prosecution case should build a save");
    let bytes = bincode::serialize(&save).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored =
        restore_save(&fixture.registry, decoded).expect("closed prosecution case should restore");
    let historical = restored
        .legal()
        .get_prosecution_case(first)
        .expect("closed prosecution case should survive restore");
    assert_eq!(historical.status(), ProsecutionCaseStatus::Closed);
    assert_eq!(historical.resolved_at(), Some(restored.now()));
    assert!(historical.resolution_information().is_some());
    assert!(historical.resolution_report().is_some());
    assert!(
        restored
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none()
    );
    assert_eq!(
        restored
            .legal()
            .get_arrest(fixture.arrest)
            .expect("originating arrest should persist through save")
            .status(),
        crate::legal::ArrestStatus::Released
    );

    let second = validate_open_prosecution_case(&restored, opening_draft(&fixture))
        .expect("terminal case should permit later reconsideration")
        .commit(&mut restored)
        .expect("reconsidered prosecution case should commit");
    assert_ne!(first, second);
    assert_eq!(
        restored
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .expect("new prosecution review should own open index")
            .id(),
        second
    );
    assert_eq!(
        restored
            .legal()
            .prosecution_cases()
            .filter(|record| record.arrest() == fixture.arrest)
            .count(),
        2
    );
    validate_state(&restored).expect("reconsidered prosecution state should validate");
    validate_invariants(&restored);
}

#[test]
fn prosecution_resolution_token_stales_after_new_referral_without_partial_resolution() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let stale_resolution = validate_decline_prosecution_case(&fixture.state, case)
        .expect("decline should initially validate");
    validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("supplement should validate before terminal disposition")
    .commit(&mut fixture.state)
    .expect("supplement should commit before stale decline token");

    assert_eq!(
        stale_resolution
            .commit(&mut fixture.state)
            .expect_err("case mutation must stale prior disposition token"),
        ProsecutionError::StaleProsecutionCase {
            case,
            expected: 1,
            found: 2,
        }
    );
    let record = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("case should remain after stale resolution rejection");
    assert_eq!(record.status(), ProsecutionCaseStatus::Reviewing);
    assert_eq!(record.resolved_at(), None);
    assert_eq!(record.resolution_information(), None);
    assert_eq!(record.resolution_report(), None);
    assert!(
        fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_some_and(|open| open.id() == case)
    );
    validate_state(&fixture.state).expect("stale disposition rejection should be atomic");
    validate_invariants(&fixture.state);
}

#[test]
fn detained_prosecutor_releases_review_for_deterministic_office_restaffing() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
    let backup = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Backup Prosecutor".to_owned(),
            organization: Some(fixture.office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(70))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("backup prosecutor fixture should validate");
    let lead_investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Prosecutor misconduct inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.lead)]),
        },
    )
    .expect("lead investigation should validate")
    .commit(&mut fixture.state)
    .expect("lead investigation should commit");
    let lead_evidence = add_evidence(
        &mut fixture.state,
        fixture.police,
        lead_investigation,
        fixture.lead,
        EvidenceKind::Document,
    );
    let lead_corroboration = add_evidence(
        &mut fixture.state,
        fixture.police,
        lead_investigation,
        fixture.lead,
        EvidenceKind::KnownAssociation,
    );
    let lead_arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.lead,
            investigation: lead_investigation,
            evidence: BTreeSet::from([lead_evidence, lead_corroboration]),
        },
    )
    .expect("lead prosecutor may be arrested without freezing unrelated prosecution work")
    .commit(&mut fixture.state)
    .expect("lead arrest should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist")
            .assigned_prosecutor(),
        None,
        "custody must release the current prosecution assignment"
    );
    validate_state(&fixture.state)
        .expect("unstaffed reviewing case should remain structurally valid");
    validate_invariants(&fixture.state);

    let error = match validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    ) {
        Ok(_) => panic!("an unstaffed prosecution case must not perform new work"),
        Err(error) => error,
    };
    assert_eq!(error, ProsecutionError::CaseUnstaffed { case });
    assert_eq!(
        validate_decline_prosecution_case(&fixture.state, case)
            .err()
            .expect("unstaffed case must not resolve"),
        ProsecutionError::CaseUnstaffed { case }
    );

    let staffed = apply_autonomous_prosecution_staffing(&mut fixture.state)
        .expect("office should restaff an unassigned review");
    assert_eq!(staffed, vec![(case, backup)]);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist")
            .assigned_prosecutor(),
        Some(backup)
    );
    validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("replacement prosecutor should continue the review")
    .commit(&mut fixture.state)
    .expect("supplement should commit under replacement prosecutor");
    let latest_referral = fixture
        .state
        .legal()
        .get_prosecution_case(case)
        .expect("case should persist")
        .referrals()
        .iter()
        .copied()
        .max()
        .and_then(|id| fixture.state.legal().get_prosecution_referral(id))
        .expect("supplemental referral should persist");
    assert_eq!(latest_referral.prosecutor(), backup);
    validate_release_arrest(&fixture.state, lead_arrest)
        .expect("former lead detention should remain independently releasable")
        .commit(&mut fixture.state)
        .expect("former lead release should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("case should persist")
            .assigned_prosecutor(),
        Some(backup),
        "releasing the former lead must not steal the replacement's assignment"
    );
    validate_state(&fixture.state).expect("restaffed prosecution state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn arrest_stales_when_prosecutor_acquires_review_after_custody_preflight() {
    let mut fixture = fixture();
    let misconduct = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Late prosecutor assignment custody test".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.lead)]),
        },
    )
    .expect("prosecutor misconduct investigation should validate")
    .commit(&mut fixture.state)
    .expect("prosecutor misconduct investigation should commit");
    let misconduct_evidence = add_evidence(
        &mut fixture.state,
        fixture.police,
        misconduct,
        fixture.lead,
        EvidenceKind::Document,
    );
    let misconduct_corroboration = add_evidence(
        &mut fixture.state,
        fixture.police,
        misconduct,
        fixture.lead,
        EvidenceKind::KnownAssociation,
    );
    let stale_arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.lead,
            investigation: misconduct,
            evidence: BTreeSet::from([misconduct_evidence, misconduct_corroboration]),
        },
    )
    .expect("arrest should validate before the prosecutor acquires review work");

    let case = open_case(&mut fixture);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist")
            .assigned_prosecutor(),
        Some(fixture.lead)
    );
    let error = stale_arrest
        .commit(&mut fixture.state)
        .expect_err("new prosecution responsibility must stale the custody preflight");
    assert!(matches!(
        error,
        ArrestError::ProsecutionStaffing(
            ProsecutionStaffingError::DetentionAssignmentsChanged { prosecutor }
        ) if prosecutor == fixture.lead
    ));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.lead)
            .is_none(),
        "stale custody must not partially insert an arrest"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("rejected custody must preserve the prosecution case")
            .assigned_prosecutor(),
        Some(fixture.lead),
        "rejected custody must not release the newly acquired prosecution assignment"
    );
    validate_state(&fixture.state).expect("stale custody rejection must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn private_legal_services_and_generic_legal_authority_cannot_act_as_prosecutor_office() {
    for kind in [
        OrganizationKind::LegalServices,
        OrganizationKind::LegalAuthority,
    ] {
        let mut fixture = fixture();
        let invalid_office = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: format!("Invalid prosecution office {kind:?}"),
                kind,
            },
        )
        .expect("invalid prosecution fixture organization should still be creatable");
        let invalid_lead = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "Invalid Prosecutor".to_owned(),
                organization: Some(invalid_office),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(80))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("invalid lead fixture should validate as a character");
        let error = match validate_open_prosecution_case(
            &fixture.state,
            ProsecutionCaseDraft {
                arrest: fixture.arrest,
                prosecutor_office: invalid_office,
                prosecutor: invalid_lead,
                evidence: arrest_evidence_set(&fixture),
            },
        ) {
            Ok(_) => panic!("non-prosecutor institution must not open prosecution case"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ProsecutionError::InvalidProsecutorOffice(invalid_office)
        );
        validate_state(&fixture.state).expect("rejected prosecutor office should preserve state");
        validate_invariants(&fixture.state);
    }
}
