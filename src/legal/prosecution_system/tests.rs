//! Focused tests for prosecution referral, review, and resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::registry::Registry;
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
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
    supplemental_evidence: EvidenceId,
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
    let arrest = validate_arrest(
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([arrest_evidence]),
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
        supplemental_evidence,
    }
}

fn opening_draft(fixture: &Fixture) -> ProsecutionCaseDraft {
    ProsecutionCaseDraft {
        arrest: fixture.arrest,
        prosecutor_office: fixture.office,
        lead_prosecutor: fixture.lead,
        evidence: BTreeSet::from([fixture.arrest_evidence]),
    }
}

fn open_case(fixture: &mut Fixture) -> ProsecutionCaseId {
    validate_open_prosecution_case(&fixture.state, opening_draft(fixture))
        .expect("prosecution case should validate")
        .commit(&mut fixture.state)
        .expect("prosecution case should commit")
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
    assert_eq!(record.lead_prosecutor(), fixture.lead);
    assert_eq!(
        record.evidence(),
        &BTreeSet::from([fixture.arrest_evidence])
    );
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
        &BTreeSet::from([fixture.arrest_evidence, fixture.supplemental_evidence])
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
fn initial_referral_must_include_every_evidence_record_that_supported_arrest() {
    let fixture = fixture();
    let error = match validate_open_prosecution_case(
        &fixture.state,
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            lead_prosecutor: fixture.lead,
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
            lead_prosecutor: second_lead,
            evidence: BTreeSet::from([fixture.arrest_evidence]),
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
            .prosecution_cases_for_arrest(fixture.arrest)
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
            .prosecution_cases_for_arrest(fixture.arrest)
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
fn detained_lead_keeps_formal_case_assignment_but_cannot_refer_new_evidence() {
    let mut fixture = fixture();
    let case = open_case(&mut fixture);
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
    let lead_arrest = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.lead,
            investigation: lead_investigation,
            evidence: BTreeSet::from([lead_evidence]),
        },
    )
    .expect("lead prosecutor may be arrested without erasing formal case assignment")
    .commit(&mut fixture.state)
    .expect("lead arrest should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist")
            .lead_prosecutor(),
        fixture.lead
    );
    validate_state(&fixture.state)
        .expect("detained lead should leave formal prosecution case structurally valid");
    validate_invariants(&fixture.state);

    let error = match validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    ) {
        Ok(_) => panic!("detained lead must not perform new prosecutorial work"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        ProsecutionError::DetainedLeadProsecutor(fixture.lead)
    );
    assert_eq!(
        validate_decline_prosecution_case(&fixture.state, case)
            .err()
            .expect("detained lead must not resolve prosecution case"),
        ProsecutionError::DetainedLeadProsecutor(fixture.lead)
    );

    validate_release_arrest(&fixture.state, lead_arrest)
        .expect("lead detention should release")
        .commit(&mut fixture.state)
        .expect("lead release should commit");
    validate_supplement_prosecution_case(
        &fixture.state,
        ProsecutionReferralDraft {
            prosecution_case: case,
            evidence: BTreeSet::from([fixture.supplemental_evidence]),
        },
    )
    .expect("released lead should resume prosecutorial work")
    .commit(&mut fixture.state)
    .expect("supplement should commit after lead release");
    validate_state(&fixture.state).expect("released lead prosecution state should validate");
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
                lead_prosecutor: invalid_lead,
                evidence: BTreeSet::from([fixture.arrest_evidence]),
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
