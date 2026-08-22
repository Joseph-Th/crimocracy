//! Focused tests for representation retention, execution, and automatic legal support.

use super::*;
use crate::build_registry;
use crate::contacts::contact_system::{
    validate_establish_contact, validate_terminate_contact, ContactError, InstitutionalContactDraft,
};
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::simulation::run_tick;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{BudgetAuthority, BudgetPeriod, MandateDraft};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{FinancialAccountDraft, LedgerPosting};
use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::registry::Registry;
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::set_policy;
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character,
};
use crate::world::PolicySetting;
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    sponsor: OrganizationId,
    handler: CharacterId,
    defendant: CharacterId,
    firm: OrganizationId,
    counsel: CharacterId,
    contact: ContactId,
    arrest: ArrestId,
    payer: FinancialAccountId,
    provider: FinancialAccountId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
}

fn relationship() -> RelationshipDimensions {
    RelationshipDimensions {
        trust: level(70),
        respect: level(65),
        fear: level(0),
        affection: level(20),
        dependence: level(30),
        resentment: level(0),
        debt: level(15),
    }
}

fn fixture() -> Fixture {
    fixture_with_counsel_institution(OrganizationKind::LegalServices)
}

fn fixture_with_counsel_institution(counsel_kind: OrganizationKind) -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0x1A77_0A93);
    let sponsor = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "North Ward Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal sponsor should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "North Ward Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police authority should validate");
    let firm = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Marchetti & Vale".to_owned(),
            kind: counsel_kind,
        },
    )
    .expect("counsel institution should validate");
    let handler = insert_character(
        &mut state,
        CharacterDraft {
            name: "Legal Liaison".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("contact handler should validate");
    let defendant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Arrested Associate".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("defendant should validate");
    let counsel = insert_character(
        &mut state,
        CharacterDraft {
            name: "Eleanor Vale".to_owned(),
            organization: Some(firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(88))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("counsel should validate");
    validate_set_relationship(&state, handler, counsel, relationship())
        .expect("lawyer relationship should validate")
        .commit(&mut state);
    let contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: counsel,
        },
    )
    .expect("legal contact should validate")
    .commit(&mut state)
    .expect("legal contact should commit");

    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "North Ward conspiracy inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(defendant)]),
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
            subject: EntityRef::Character(defendant),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(&mut state)
    .expect("case evidence should commit");
    let arrest = validate_arrest(
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");

    let payer = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("payer account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    let provider = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(firm),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("provider account should validate");
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Opening legal reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-50_000),
                },
                LedgerPosting {
                    account: payer,
                    amount: Money::from_cents(50_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("opening reserve should validate")
    .commit(&mut state)
    .expect("opening reserve should commit");

    Fixture {
        registry,
        state,
        sponsor,
        handler,
        defendant,
        firm,
        counsel,
        contact,
        arrest,
        payer,
        provider,
    }
}

fn representation_draft(
    fixture: &Fixture,
    fee_cents: i64,
    authorization: Option<MandateAuthority>,
) -> LegalRepresentationDraft {
    LegalRepresentationDraft {
        arrest: fixture.arrest,
        sponsor: fixture.sponsor,
        contact: fixture.contact,
        fee: Money::from_cents(fee_cents),
        payer_account: fixture.payer,
        provider_account: fixture.provider,
        authorization,
    }
}

fn retain(
    fixture: &mut Fixture,
    fee_cents: i64,
    authorization: Option<MandateAuthority>,
) -> LegalRepresentationId {
    validate_retain_legal_representation(
        &fixture.state,
        representation_draft(fixture, fee_cents, authorization),
    )
    .expect("legal representation should validate")
    .commit(&mut fixture.state)
    .expect("legal representation should commit")
}

#[test]
fn automatic_legal_support_policy_retains_counsel_through_the_tick() {
    let mut fx = fixture();
    // Flip the sponsor's standing policy to automatic support through the canonical
    // owner path; the default CaseByCase setting never acts on its own.
    set_policy(
        &fx.registry,
        &mut fx.state,
        fx.sponsor,
        PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
    )
    .expect("automatic legal-support policy should validate");
    let payer_before = fx
        .state
        .finance()
        .get_account(fx.payer)
        .expect("payer account should exist")
        .balance();

    let outcome = run_tick(&fx.registry, &mut fx.state);
    assert_eq!(
        outcome.automatic_legal_support.len(),
        1,
        "the tick must retain counsel for the detained member exactly once"
    );
    let representation = fx
        .state
        .legal()
        .active_representation_for_arrest(fx.arrest)
        .expect("automatic policy should have retained counsel");
    assert_eq!(representation.sponsor(), fx.sponsor);
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("payer account should exist")
            .balance()
            .cents(),
        payer_before.cents() - 5_000,
        "the flat authored retainer must be the only cost of the automatic path"
    );

    // A second tick must not retain again: the arrest is already represented.
    let outcome = run_tick(&fx.registry, &mut fx.state);
    assert!(outcome.automatic_legal_support.is_empty());
    validate_state(&fx.state).expect("automatic support state should remain valid");
    validate_invariants(&fx.state);

    // The default CaseByCase policy never fires on its own.
    let untouched = fixture();
    assert!(untouched
        .state
        .legal()
        .active_representation_for_arrest(untouched.arrest)
        .is_none());
}

#[test]
fn retain_legal_representation_id_exhaustion_is_atomic_and_typed() {
    // The composite retain commit allocates a ledger transaction, information, a report, and
    // then the representation ID. Exhausting the report class must abort the whole commit
    // before the ledger funds move, instead of stranding a partly-applied transaction.
    let mut fixture = fixture();
    let payer_before = fixture
        .state
        .finance()
        .get_account(fixture.payer)
        .expect("payer account should exist")
        .balance();
    let ledger_before = fixture.state.finance().transactions().count();
    let information_before = fixture.state.intelligence().information().count();
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Report, u32::MAX);

    let validated = {
        let draft = representation_draft(&fixture, 12_000, None);
        validate_retain_legal_representation(&fixture.state, draft)
            .expect("read-only validation must ignore ID exhaustion")
    };
    let error = validated
        .commit(&mut fixture.state)
        .expect_err("report exhaustion must reject the composite commit");
    assert!(
        matches!(error, LegalRepresentationError::IdExhaustion(_)),
        "expected typed ID exhaustion, got {error:?}"
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.payer)
            .expect("payer account should persist")
            .balance(),
        payer_before,
        "ledger must not move when a later ID allocation is exhausted"
    );
    assert_eq!(
        fixture.state.finance().transactions().count(),
        ledger_before,
        "no ledger transaction may be committed on a rejected composite commit"
    );
    assert_eq!(
        fixture.state.intelligence().information().count(),
        information_before,
        "no information may be recorded on a rejected composite commit"
    );
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none(),
        "no representation may be created on a rejected composite commit"
    );
    validate_state(&fixture.state).expect("rejected commit must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn retained_counsel_is_paid_indexed_reported_and_survives_save() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 12_000, None);
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("representation should persist");
    assert_eq!(record.status(), LegalRepresentationStatus::Active);
    assert_eq!(record.defendant(), fixture.defendant);
    assert_eq!(record.counsel(), fixture.counsel);
    assert_eq!(record.counsel_institution(), fixture.firm);
    assert_eq!(record.contact(), fixture.contact);
    assert_eq!(record.fee(), Money::from_cents(12_000));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.payer)
            .expect("payer should exist")
            .balance(),
        Money::from_cents(38_000)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.provider)
            .expect("provider should exist")
            .balance(),
        Money::from_cents(12_000)
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .map(|record| record.id()),
        Some(representation)
    );
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(record.report())
            .expect("retainer report should persist")
            .kind(),
        ReportKind::Legal
    );
    validate_state(&fixture.state).expect("retained-counsel state should validate");
    validate_invariants(&fixture.state);

    let save = build_save(&fixture.registry, &fixture.state)
        .expect("retained-counsel state should build a save");
    let bytes = bincode::serialize(&save).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored =
        restore_save(&fixture.registry, decoded).expect("retained-counsel state should restore");
    assert_eq!(
        restored
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .map(|record| record.id()),
        Some(representation)
    );

    validate_end_legal_representation(
        &restored,
        representation,
        LegalRepresentationEndReason::MatterConcluded,
    )
    .expect("restored representation should be endable")
    .commit(&mut restored)
    .expect("representation end should commit");
    let ended = restored
        .legal()
        .get_legal_representation(representation)
        .expect("ended representation should remain historical");
    assert_eq!(ended.status(), LegalRepresentationStatus::Ended);
    assert_eq!(
        ended.end_reason(),
        Some(LegalRepresentationEndReason::MatterConcluded)
    );
    assert!(ended.ended_information().is_some());
    assert!(ended.ended_report().is_some());
    assert!(restored
        .legal()
        .active_representation_for_arrest(fixture.arrest)
        .is_none());

    let replacement = validate_retain_legal_representation(
        &restored,
        LegalRepresentationDraft {
            arrest: fixture.arrest,
            sponsor: fixture.sponsor,
            contact: fixture.contact,
            fee: Money::from_cents(5_000),
            payer_account: fixture.payer,
            provider_account: fixture.provider,
            authorization: None,
        },
    )
    .expect("ended representation should permit later counsel retention")
    .commit(&mut restored)
    .expect("later representation should commit with fresh ID");
    assert_ne!(replacement, representation);
    assert_eq!(
        restored
            .legal()
            .representations_for_arrest(fixture.arrest)
            .count(),
        2
    );
    validate_state(&restored).expect("restored replacement-counsel state should validate");
    validate_invariants(&restored);
}

#[test]
fn active_representation_locks_contact_until_representation_ends() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 8_000, None);
    let error = validate_terminate_contact(&fixture.state, fixture.contact)
        .expect_err("active representation must retain its contact dependency");
    assert_eq!(
        error,
        ContactError::ActiveLegalRepresentation {
            contact: fixture.contact,
            representation,
        }
    );

    validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::SponsorWithdrawn,
    )
    .expect("representation end should validate")
    .commit(&mut fixture.state)
    .expect("representation end should commit");
    validate_terminate_contact(&fixture.state, fixture.contact)
        .expect("ended representation should release contact dependency")
        .commit(&mut fixture.state)
        .expect("contact termination should commit");
    validate_state(&fixture.state).expect("ended representation history should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn active_representation_survives_defendant_departure_after_release() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant detention should release")
        .commit(&mut fixture.state)
        .expect("defendant release should commit");
    validate_reassign_character(&fixture.state, fixture.defendant, None, None)
        .expect("released defendant may leave sponsoring organization")
        .commit(&mut fixture.state)
        .expect("defendant departure should commit");
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("representation should persist after defendant departure");
    assert_eq!(record.status(), LegalRepresentationStatus::Active);
    assert_eq!(record.sponsor(), fixture.sponsor);
    assert_eq!(record.defendant(), fixture.defendant);
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.defendant)
            .expect("defendant should persist")
            .organization(),
        None
    );
    validate_state(&fixture.state)
        .expect("active representation should survive post-release membership change");
    validate_invariants(&fixture.state);
}

#[test]
fn automatic_policy_concludes_representation_after_release_and_frees_the_contact() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant detention should release")
        .commit(&mut fixture.state)
        .expect("defendant release should commit");

    // The next automatic-support stage concludes the now-moot matter through the canonical
    // end path so the Legal contact becomes available again instead of staying locked.
    let ended = apply_automatic_legal_support(&mut fixture.state)
        .expect("automatic legal support should resolve");
    assert!(ended.is_empty(), "retention must not rerun after release");
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("representation should persist after conclusion");
    assert_eq!(record.status(), LegalRepresentationStatus::Ended);
    assert_eq!(
        record.end_reason(),
        Some(LegalRepresentationEndReason::MatterConcluded)
    );
    validate_terminate_contact(&fixture.state, fixture.contact)
        .expect("concluded representation should free its contact")
        .commit(&mut fixture.state)
        .expect("contact termination should commit");
    validate_state(&fixture.state).expect("concluded representation state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn stale_retainer_after_contact_termination_is_atomic() {
    let mut fixture = fixture();
    let validated = validate_retain_legal_representation(
        &fixture.state,
        representation_draft(&fixture, 9_000, None),
    )
    .expect("initial retainer should validate");
    validate_terminate_contact(&fixture.state, fixture.contact)
        .expect("unused contact should terminate")
        .commit(&mut fixture.state)
        .expect("contact termination should commit");

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("terminated contact must stale older retainer validation");
    assert!(matches!(
        error,
        LegalRepresentationError::StaleContact { .. }
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.payer)
            .expect("payer should exist")
            .balance(),
        Money::from_cents(50_000)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.provider)
            .expect("provider should exist")
            .balance(),
        Money::ZERO
    );
    assert!(fixture
        .state
        .legal()
        .active_representation_for_arrest(fixture.arrest)
        .is_none());
    validate_state(&fixture.state).expect("rejected stale retainer should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_legal_budget_authority_is_persisted_and_enforced() {
    let mut fixture = fixture();
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.sponsor,
            manager: fixture.handler,
            scopes: BTreeSet::from([ResponsibilityScope::Function(ResponsibilityFunction::Legal)]),
            standing_orders: BTreeMap::new(),
            budget: Some(BudgetAuthority {
                funding_account: fixture.payer,
                limit: Money::from_cents(15_000),
                period: BudgetPeriod::Weekly,
            }),
        },
    )
    .expect("legal-support mandate should validate")
    .commit(&mut fixture.state)
    .expect("legal-support mandate should commit");
    let wrong_scope = MandateAuthority {
        mandate,
        manager: fixture.handler,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
    };
    let error = match validate_retain_legal_representation(
        &fixture.state,
        representation_draft(&fixture, 10_000, Some(wrong_scope)),
    ) {
        Ok(_) => panic!("non-Legal delegated scope must not authorize counsel retention"),
        Err(error) => error,
    };
    assert_eq!(error, LegalRepresentationError::InvalidAuthorityScope);

    let authority = MandateAuthority {
        mandate,
        manager: fixture.handler,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Legal),
    };
    let representation = retain(&mut fixture, 10_000, Some(authority));
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("delegated representation should persist");
    assert_eq!(record.authorization(), Some(authority));
    let usage = fixture
        .state
        .finance()
        .get_transaction(record.payment())
        .expect("retainer payment should persist")
        .budget_usage()
        .expect("delegated retainer should persist budget usage");
    assert_eq!(usage.mandate(), mandate);
    assert_eq!(usage.manager(), fixture.handler);
    assert_eq!(usage.scope(), authority.scope);
    assert_eq!(usage.funding_account(), fixture.payer);
    assert_eq!(usage.amount(), Money::from_cents(10_000));
    validate_state(&fixture.state).expect("delegated legal-support state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn public_legal_authority_cannot_masquerade_as_private_defense_provider() {
    let fixture = fixture_with_counsel_institution(OrganizationKind::LegalAuthority);
    let error = match validate_retain_legal_representation(
        &fixture.state,
        representation_draft(&fixture, 6_000, None),
    ) {
        Ok(_) => {
            panic!("public legal authority contact must not become private defense counsel")
        }
        Err(error) => error,
    };
    assert_eq!(
        error,
        LegalRepresentationError::InvalidCounselInstitution(fixture.firm)
    );
    validate_state(&fixture.state).expect("rejected defense-provider state should validate");
    validate_invariants(&fixture.state);
}
