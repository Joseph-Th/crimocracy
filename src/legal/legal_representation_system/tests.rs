//! Focused tests for representation retention, execution, and automatic legal support.

use super::*;
use crate::build_registry;
use crate::contacts::contact_system::{
    ContactError, InstitutionalContactDraft, validate_establish_contact, validate_terminate_contact,
};
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
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
use crate::world::PolicyKind;
use crate::world::PolicySetting;
use crate::world::world_system::set_policy;
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    sponsor: OrganizationId,
    police: OrganizationId,
    handler: CharacterId,
    defendant: CharacterId,
    supervisor: Option<CharacterId>,
    firm: OrganizationId,
    counsel: CharacterId,
    contact: ContactId,
    arrest: ArrestId,
    payer: FinancialAccountId,
    provider: FinancialAccountId,
}

#[test]
fn restore_rejects_representation_predating_its_arrest_anchor() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 12_000, None);
    let retained_at = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("representation should persist")
        .retained_at();
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::ONE_MINUTE);
    let arrest = fixture
        .state
        .legal()
        .get_arrest(fixture.arrest)
        .expect("arrest should persist")
        .clone();
    assert_eq!(arrest.arrested_at(), retained_at);
    let mut corrupted = arrest_wire(&arrest);
    corrupted.arrested_at = fixture.state.now();

    let error = restore_save(
        &fixture.registry,
        replace_serialized_arrest(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid represented arrest should save before chronology corruption"),
            &arrest,
            &corrupted,
        ),
    )
    .expect_err("representation cannot predate the arrest it was retained to answer");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidLegalRepresentation {
                representation: invalid
            }
        ) if invalid == representation
    ));
}

#[derive(Clone, Serialize)]
struct ArrestRecordWire {
    id: ArrestId,
    character: CharacterId,
    authority: OrganizationId,
    investigation: crate::core::id::InvestigationId,
    evidence: BTreeSet<crate::core::id::EvidenceId>,
    arrested_at: SimTime,
    released_at: Option<SimTime>,
    status: crate::legal::ArrestStatus,
    version: u32,
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

#[test]
fn automatic_legal_support_aggregates_split_organization_liquidity() {
    let mut fx = fixture();
    let second_payer = insert_account(
        &mut fx.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fx.sponsor),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("second sponsor reserve should validate");
    let parked = insert_account(
        &mut fx.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fx.sponsor),
            kind: AccountKind::Settlement,
        },
    )
    .expect("non-spendable parking account should validate");
    validate_record_transaction(
        &fx.state,
        LedgerTransactionDraft {
            occurred_at: fx.state.now(),
            memo: "Partition legal reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fx.payer,
                    amount: Money::from_cents(-45_000),
                },
                LedgerPosting {
                    account: parked,
                    amount: Money::from_cents(45_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("reserve partition should validate")
    .commit(&mut fx.state)
    .expect("reserve partition should commit");
    validate_record_transaction(
        &fx.state,
        LedgerTransactionDraft {
            occurred_at: fx.state.now(),
            memo: "Split legal liquidity".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fx.payer,
                    amount: Money::from_cents(-2_500),
                },
                LedgerPosting {
                    account: second_payer,
                    amount: Money::from_cents(2_500),
                },
            ],
            authorization: None,
        },
    )
    .expect("liquidity split should validate")
    .commit(&mut fx.state)
    .expect("liquidity split should commit");
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("first payer should persist")
            .balance(),
        Money::from_cents(2_500)
    );
    assert_eq!(
        fx.state
            .finance()
            .get_account(second_payer)
            .expect("second payer should persist")
            .balance(),
        Money::from_cents(2_500)
    );

    set_policy(
        &fx.registry,
        &mut fx.state,
        fx.sponsor,
        PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
    )
    .expect("automatic legal-support policy should validate");
    let retained = apply_automatic_legal_support(&mut fx.state)
        .expect("aggregate sponsor liquidity should fund automatic counsel");
    assert_eq!(retained.len(), 1);
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("first payer should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fx.state
            .finance()
            .get_account(second_payer)
            .expect("second payer should persist")
            .balance(),
        Money::ZERO
    );
    let representation = fx
        .state
        .legal()
        .get_legal_representation(retained[0])
        .expect("representation should persist");
    let payment = fx
        .state
        .finance()
        .get_transaction(representation.payment())
        .expect("retainer payment should persist");
    assert_eq!(payment.postings().len(), 3);
    assert!(payment.postings().iter().any(|posting| {
        posting.account == fx.payer && posting.amount == Money::from_cents(-2_500)
    }));
    assert!(payment.postings().iter().any(|posting| {
        posting.account == second_payer && posting.amount == Money::from_cents(-2_500)
    }));
    validate_state(&fx.state).expect("split-liquidity legal support should validate");
    validate_invariants(&fx.state);
}

#[test]
fn mandate_automatic_legal_support_respects_exhausted_budget_window() {
    let mut fx = fixture_with_options(OrganizationKind::LegalServices, true);
    let supervisor = fx
        .supervisor
        .expect("supervised fixture should carry a boss");
    let mandate = validate_assign_mandate(
        &fx.state,
        MandateDraft {
            organization: fx.sponsor,
            manager: supervisor,
            scopes: BTreeSet::from([ResponsibilityScope::Function(ResponsibilityFunction::Legal)]),
            standing_orders: BTreeMap::from([(
                PolicyKind::AssociateLegalSupport,
                PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
            )]),
            budget: Some(BudgetAuthority {
                funding_account: fx.payer,
                limit: Money::from_cents(5_000),
                period: BudgetPeriod::Weekly,
            }),
        },
    )
    .expect("budgeted automatic-support mandate should validate")
    .commit(&mut fx.state)
    .expect("budgeted automatic-support mandate should commit");
    let authority = MandateAuthority {
        mandate,
        manager: supervisor,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Legal),
    };
    validate_record_transaction(
        &fx.state,
        LedgerTransactionDraft {
            occurred_at: fx.state.now(),
            memo: "Consume delegated legal budget before automatic support".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fx.payer,
                    amount: Money::from_cents(-5_000),
                },
                LedgerPosting {
                    account: fx.provider,
                    amount: Money::from_cents(5_000),
                },
            ],
            authorization: Some(authority),
        },
    )
    .expect("fixture legal spend should consume the mandate budget")
    .commit(&mut fx.state)
    .expect("fixture legal spend should commit");
    let payer_before = fx
        .state
        .finance()
        .get_account(fx.payer)
        .expect("payer should persist")
        .balance();

    assert!(
        apply_automatic_legal_support(&mut fx.state)
            .expect("an exhausted budget is ordinary unavailability, not a failed legal pass")
            .is_empty()
    );
    assert!(
        fx.state
            .legal()
            .active_representation_for_arrest(fx.arrest)
            .is_none()
    );
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("payer should persist")
            .balance(),
        payer_before
    );
    let usage =
        crate::finance::finance_system::resolve_budget_usage(&fx.state, mandate, fx.state.now())
            .expect("exhausted mandate budget should remain resolvable");
    assert_eq!(usage.used, Money::from_cents(5_000));
    assert_eq!(usage.remaining, Money::ZERO);
    validate_state(&fx.state).expect("exhausted-budget legal-support state should remain valid");
    validate_invariants(&fx.state);
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
    fixture_with_options(counsel_kind, false)
}

/// Full retention fixture. With `supervised_defendant`, the arrested associate reports to an
/// inserted street boss whose mandate standing orders can be exercised by tests.
fn fixture_with_options(counsel_kind: OrganizationKind, supervised_defendant: bool) -> Fixture {
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
    let supervisor = supervised_defendant.then(|| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: "Street Boss".to_owned(),
                organization: Some(sponsor),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("street boss should validate")
    });
    let defendant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Arrested Associate".to_owned(),
            organization: Some(sponsor),
            supervisor,
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
        .commit(&mut state)
        .expect("relationship should commit");
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
        police,
        handler,
        defendant,
        supervisor,
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
        payer_accounts: BTreeSet::from([fixture.payer]),
        provider_account: fixture.provider,
        authorization,
        origin: crate::legal::LegalRepresentationOrigin::DirectRetention,
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

fn arrest_draft_for_character(
    fixture: &mut Fixture,
    character: CharacterId,
    title: &str,
) -> ArrestDraft {
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: title.to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(character)]),
        },
    )
    .expect("custody test investigation should validate")
    .commit(&mut fixture.state)
    .expect("custody test investigation should commit");
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("custody test evidence should validate")
    .commit(&mut fixture.state)
    .expect("custody test evidence should commit");
    ArrestDraft {
        character,
        investigation,
        evidence: BTreeSet::from([evidence]),
    }
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
    assert!(
        untouched
            .state
            .legal()
            .active_representation_for_arrest(untouched.arrest)
            .is_none()
    );
}

#[test]
fn automatic_legal_support_surfaces_commit_failure_instead_of_silently_skipping_counsel() {
    let mut fx = fixture();
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
        .expect("payer account should persist")
        .balance();
    fx.state.ids.set_next_raw_for_test(IdKind::Report, u32::MAX);

    let error = apply_automatic_legal_support(&mut fx.state)
        .expect_err("automatic support must surface a canonical commit failure");
    assert!(matches!(error, LegalRepresentationError::IdExhaustion(_)));
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("payer account should persist")
            .balance(),
        payer_before,
        "failed automatic retention must not move retainer funds"
    );
    assert!(
        fx.state
            .legal()
            .active_representation_for_arrest(fx.arrest)
            .is_none(),
        "failed automatic retention must not manufacture representation state"
    );
}

#[test]
fn automatic_legal_support_skips_detained_counsel_for_a_later_viable_channel() {
    let mut fx = fixture();
    set_policy(
        &fx.registry,
        &mut fx.state,
        fx.sponsor,
        PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
    )
    .expect("automatic legal-support policy should validate");

    let replacement_counsel = insert_character(
        &mut fx.state,
        CharacterDraft {
            name: "Clara Voss".to_owned(),
            organization: Some(fx.firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(82))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("replacement counsel should validate");
    validate_set_relationship(&fx.state, fx.handler, replacement_counsel, relationship())
        .expect("replacement counsel relationship should validate")
        .commit(&mut fx.state)
        .expect("relationship should commit");
    let replacement_contact = validate_establish_contact(
        &fx.state,
        InstitutionalContactDraft {
            sponsor: fx.sponsor,
            handler: fx.handler,
            contact: replacement_counsel,
        },
    )
    .expect("replacement legal contact should validate")
    .commit(&mut fx.state)
    .expect("replacement legal contact should commit");
    assert!(replacement_contact > fx.contact);

    let police = fx
        .state
        .legal()
        .get_arrest(fx.arrest)
        .expect("fixture arrest should persist")
        .authority();
    let counsel_case = validate_open_investigation(
        &fx.state,
        InvestigationDraft {
            owner: police,
            title: "Counsel detention inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fx.counsel)]),
        },
    )
    .expect("counsel investigation should validate")
    .commit(&mut fx.state)
    .expect("counsel investigation should commit");
    let counsel_evidence = validate_add_evidence(
        &fx.state,
        EvidenceDraft {
            investigation: counsel_case,
            custodian: police,
            subject: EntityRef::Character(fx.counsel),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fx.state.now(),
        },
    )
    .expect("counsel evidence should validate")
    .commit(&mut fx.state)
    .expect("counsel evidence should commit");
    validate_arrest(
        &fx.state,
        ArrestDraft {
            character: fx.counsel,
            investigation: counsel_case,
            evidence: BTreeSet::from([counsel_evidence]),
        },
    )
    .expect("counsel detention should validate")
    .commit(&mut fx.state)
    .expect("counsel detention should commit");

    let retained = apply_automatic_legal_support(&mut fx.state)
        .expect("automatic support should continue past an unavailable older channel");
    assert_eq!(retained.len(), 1);
    let representation = fx
        .state
        .legal()
        .get_legal_representation(retained[0])
        .expect("replacement representation should persist");
    assert_eq!(representation.contact(), replacement_contact);
    assert_eq!(representation.counsel(), replacement_counsel);
    validate_state(&fx.state).expect("replacement legal channel state should validate");
    validate_invariants(&fx.state);
}

#[test]
fn automatic_legal_support_prefers_stronger_later_counsel() {
    let mut fx = fixture();
    set_policy(
        &fx.registry,
        &mut fx.state,
        fx.sponsor,
        PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
    )
    .expect("automatic legal-support policy should validate");

    let stronger_counsel = insert_character(
        &mut fx.state,
        CharacterDraft {
            name: "Margaret Shaw".to_owned(),
            organization: Some(fx.firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(96))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("stronger counsel should validate");
    validate_set_relationship(&fx.state, fx.handler, stronger_counsel, relationship())
        .expect("stronger counsel relationship should validate")
        .commit(&mut fx.state)
        .expect("relationship should commit");
    let stronger_contact = validate_establish_contact(
        &fx.state,
        InstitutionalContactDraft {
            sponsor: fx.sponsor,
            handler: fx.handler,
            contact: stronger_counsel,
        },
    )
    .expect("stronger legal contact should validate")
    .commit(&mut fx.state)
    .expect("stronger legal contact should commit");
    assert!(
        stronger_contact > fx.contact,
        "fixture must make the stronger lawyer the newer contact"
    );

    let retained = apply_automatic_legal_support(&mut fx.state)
        .expect("automatic support should select among viable lawyers");
    assert_eq!(retained.len(), 1);
    let representation = fx
        .state
        .legal()
        .get_legal_representation(retained[0])
        .expect("automatic representation should persist");
    assert_eq!(representation.contact(), stronger_contact);
    assert_eq!(representation.counsel(), stronger_counsel);
    validate_state(&fx.state).expect("competence-ranked automatic support should remain valid");
    validate_invariants(&fx.state);
}

#[test]
fn mandate_standing_order_governs_automatic_legal_support_for_the_supervised() {
    let mut fx = fixture_with_options(OrganizationKind::LegalServices, true);
    let supervisor = fx
        .supervisor
        .expect("supervised fixture should carry a boss");
    // The organization keeps its default CaseByCase policy; only the supervisor's mandate
    // standing order makes support automatic for the crew under them.
    validate_assign_mandate(
        &fx.state,
        MandateDraft {
            organization: fx.sponsor,
            manager: supervisor,
            scopes: BTreeSet::from([ResponsibilityScope::Function(ResponsibilityFunction::Legal)]),
            standing_orders: BTreeMap::from([(
                PolicyKind::AssociateLegalSupport,
                PolicySetting::AssociateLegalSupport(crate::world::LegalSupportPolicy::Automatic),
            )]),
            budget: Some(BudgetAuthority {
                funding_account: fx.payer,
                limit: Money::from_cents(5_000),
                period: BudgetPeriod::Weekly,
            }),
        },
    )
    .expect("mandate with legal-support standing order should validate")
    .commit(&mut fx.state)
    .expect("mandate should commit");

    // Without a mandate override this default-policy organization would never act.
    let untouched = fixture();
    assert!(
        untouched
            .state
            .legal()
            .active_representation_for_arrest(untouched.arrest)
            .is_none()
    );

    let outcome = run_tick(&fx.registry, &mut fx.state);
    assert_eq!(
        outcome.automatic_legal_support.len(),
        1,
        "the supervised defendant's retention must follow the mandate standing order"
    );
    let representation = fx
        .state
        .legal()
        .active_representation_for_arrest(fx.arrest)
        .expect("mandate-driven support should persist");
    let authority = MandateAuthority {
        mandate: fx
            .state
            .delegation()
            .active_for_manager(supervisor)
            .expect("supervisor mandate should remain active")
            .id(),
        manager: supervisor,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Legal),
    };
    assert_eq!(representation.authorization(), Some(authority));
    let usage = fx
        .state
        .finance()
        .get_transaction(representation.payment())
        .expect("automatic mandate payment should persist")
        .budget_usage()
        .expect("automatic mandate payment must consume delegated budget");
    assert_eq!(usage.mandate(), authority.mandate);
    assert_eq!(usage.funding_account(), fx.payer);
    assert_eq!(usage.amount(), Money::from_cents(5_000));
    validate_state(&fx.state).expect("mandate-driven support state should remain valid");
    validate_invariants(&fx.state);
}

#[test]
fn mandate_automatic_legal_support_cannot_spend_without_legal_budget_authority() {
    let mut fx = fixture_with_options(OrganizationKind::LegalServices, true);
    let supervisor = fx
        .supervisor
        .expect("supervised fixture should carry a boss");
    let assign = |fx: &mut Fixture, scopes: BTreeSet<ResponsibilityScope>, budget| {
        validate_assign_mandate(
            &fx.state,
            MandateDraft {
                organization: fx.sponsor,
                manager: supervisor,
                scopes,
                standing_orders: BTreeMap::from([(
                    PolicyKind::AssociateLegalSupport,
                    PolicySetting::AssociateLegalSupport(
                        crate::world::LegalSupportPolicy::Automatic,
                    ),
                )]),
                budget,
            },
        )
        .expect("automatic-support mandate fixture should validate")
        .commit(&mut fx.state)
        .expect("automatic-support mandate fixture should commit")
    };

    let payer_before = fx
        .state
        .finance()
        .get_account(fx.payer)
        .expect("payer should persist")
        .balance();
    let no_budget = assign(
        &mut fx,
        BTreeSet::from([ResponsibilityScope::Function(ResponsibilityFunction::Legal)]),
        None,
    );
    assert!(
        apply_automatic_legal_support(&mut fx.state)
            .expect("missing delegated budget is an unavailable prerequisite, not state drift")
            .is_empty()
    );
    assert!(
        fx.state
            .legal()
            .active_representation_for_arrest(fx.arrest)
            .is_none()
    );
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("payer should persist")
            .balance(),
        payer_before
    );
    crate::delegation::delegation_system::validate_revoke_mandate(&fx.state, no_budget)
        .expect("unused mandate should be revocable")
        .commit(&mut fx.state)
        .expect("unused mandate revocation should commit");

    let payer = fx.payer;
    assign(
        &mut fx,
        BTreeSet::from([ResponsibilityScope::Function(
            ResponsibilityFunction::Personnel,
        )]),
        Some(BudgetAuthority {
            funding_account: payer,
            limit: Money::from_cents(50_000),
            period: BudgetPeriod::Weekly,
        }),
    );
    assert!(
        apply_automatic_legal_support(&mut fx.state)
            .expect("non-Legal mandate scope is not legal spending authority")
            .is_empty()
    );
    assert!(
        fx.state
            .legal()
            .active_representation_for_arrest(fx.arrest)
            .is_none()
    );
    assert_eq!(
        fx.state
            .finance()
            .get_account(fx.payer)
            .expect("payer should persist")
            .balance(),
        payer_before
    );
    validate_state(&fx.state).expect("rejected delegated automatic support should remain valid");
    validate_invariants(&fx.state);
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
    assert!(
        restored
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none()
    );

    let replacement = validate_retain_legal_representation(
        &restored,
        LegalRepresentationDraft {
            arrest: fixture.arrest,
            sponsor: fixture.sponsor,
            contact: fixture.contact,
            fee: Money::from_cents(5_000),
            payer_accounts: BTreeSet::from([fixture.payer]),
            provider_account: fixture.provider,
            authorization: None,
            origin: crate::legal::LegalRepresentationOrigin::DirectRetention,
        },
    )
    .expect("ended representation should permit later counsel retention")
    .commit(&mut restored)
    .expect("later representation should commit with fresh ID");
    assert_ne!(replacement, representation);
    assert_eq!(
        restored
            .legal()
            .legal_representations()
            .filter(|record| record.arrest() == fixture.arrest)
            .count(),
        2
    );
    validate_state(&restored).expect("restored replacement-counsel state should validate");
    validate_invariants(&restored);
}

#[test]
fn retained_representation_rejects_matching_but_unauthored_persisted_summary() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 12_000, None);
    let summary = {
        let record = fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("representation should persist");
        fixture
            .state
            .intelligence()
            .get_information(record.information())
            .expect("retained representation information should persist")
            .summary()
            .to_owned()
    };
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid retained representation should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 2);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten retained narrative must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidLegalRepresentation {
                    representation: invalid,
                }
            ) if invalid == representation
        ),
        "expected invalid representation, got {error:?}"
    );
}

#[test]
fn ended_representation_rejects_matching_but_unauthored_persisted_summary() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 12_000, None);
    validate_end_legal_representation(
        &fixture.state,
        representation,
        LegalRepresentationEndReason::CounselUnavailable,
    )
    .expect("representation ending should validate")
    .commit(&mut fixture.state)
    .expect("representation ending should commit");
    let summary = {
        let record = fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("ended representation should persist");
        let information = record
            .ended_information()
            .expect("ended representation should retain ending information");
        fixture
            .state
            .intelligence()
            .get_information(information)
            .expect("ended representation information should persist")
            .summary()
            .to_owned()
    };
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid ended representation should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 2);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten ending narrative must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidLegalRepresentation {
                    representation: invalid,
                }
            ) if invalid == representation
        ),
        "expected invalid representation, got {error:?}"
    );
}

#[test]
fn arresting_retained_counsel_ends_representation_before_custody() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 8_000, None);
    let counsel = fixture.counsel;
    let draft =
        arrest_draft_for_character(&mut fixture, counsel, "Counsel obstruction investigation");

    let counsel_arrest = validate_arrest(&fixture.state, draft)
        .expect("counsel custody should validate with representation preemption")
        .commit(&mut fixture.state)
        .expect("counsel custody should end representation atomically");

    assert_eq!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(counsel)
            .map(|arrest| arrest.id()),
        Some(counsel_arrest)
    );
    let ended = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("ended representation should remain historical");
    assert_eq!(ended.status(), LegalRepresentationStatus::Ended);
    assert_eq!(
        ended.end_reason(),
        Some(LegalRepresentationEndReason::CounselUnavailable)
    );
    assert!(ended.ended_information().is_some());
    assert!(ended.ended_report().is_some());
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none(),
        "detained counsel cannot remain the active lawyer on another detainee's matter"
    );
    validate_state(&fixture.state).expect("counsel-custody state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn counsel_arrest_token_stales_when_representation_is_retained_after_validation() {
    let mut fixture = fixture();
    let counsel = fixture.counsel;
    let draft = arrest_draft_for_character(
        &mut fixture,
        counsel,
        "Counsel stale-preflight investigation",
    );
    let validated = validate_arrest(&fixture.state, draft)
        .expect("counsel arrest should validate before a matter is retained");
    let representation = retain(&mut fixture, 8_000, None);

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("new representation must stale the earlier custody preflight");
    assert_eq!(
        error,
        crate::legal::arrest_system::ArrestError::LegalRepresentation(
            LegalRepresentationError::DetentionRepresentationsChanged { counsel }
        )
    );
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(counsel)
            .is_none(),
        "stale custody token must not partially arrest counsel"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("newly retained representation should persist")
            .status(),
        LegalRepresentationStatus::Active
    );
    validate_state(&fixture.state).expect("rejected stale custody state should validate");
    validate_invariants(&fixture.state);
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
    let mut draft = representation_draft(&fixture, 7_500, None);
    // The sweep may only conclude matters its own governance retained.
    draft.origin = crate::legal::LegalRepresentationOrigin::AutomaticPolicy;
    let representation = validate_retain_legal_representation(&fixture.state, draft)
        .expect("automatic retention should validate")
        .commit(&mut fixture.state)
        .expect("automatic retention should commit");
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
fn custody_sweep_never_ends_an_explicitly_retained_representation() {
    let mut fixture = fixture();
    let representation = retain(&mut fixture, 7_500, None);
    validate_release_arrest(&fixture.state, fixture.arrest)
        .expect("defendant detention should release")
        .commit(&mut fixture.state)
        .expect("defendant release should commit");

    apply_automatic_legal_support(&mut fixture.state)
        .expect("automatic legal support should resolve");
    // A directly commanded retention outlives custody: only leadership ends it.
    let record = fixture
        .state
        .legal()
        .get_legal_representation(representation)
        .expect("explicitly retained representation should persist");
    assert_eq!(record.status(), LegalRepresentationStatus::Active);
    assert_eq!(record.end_reason(), None);
    validate_state(&fixture.state).expect("swept-custody state should validate");
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
    assert!(
        fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none()
    );
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
    let second_payer = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.sponsor),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("second sponsor account should validate");
    let mut mixed_funding = representation_draft(&fixture, 10_000, Some(authority));
    mixed_funding.payer_accounts.insert(second_payer);
    assert_eq!(
        validate_retain_legal_representation(&fixture.state, mixed_funding)
            .err()
            .expect("delegated spending must not escape its designated budget account"),
        LegalRepresentationError::DelegatedFundingAccountMismatch {
            required: fixture.payer,
        }
    );
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
