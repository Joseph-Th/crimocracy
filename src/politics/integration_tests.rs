use super::*;
use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
use crate::core::campaign_victory::{VictoryError, VictoryPath, validate_claim_victory};
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::politics::political_system::{
    PoliticalError, apply_due_political_campaigns, initiative_available, validate_civic_initiative,
    validate_nominate_mayor,
};
use crate::world::{AutonomyLevel, OrganizationKind};

fn fund(state: &mut crate::AppState, owner: OrganizationId, cents: i64) {
    let source = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let destination = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Political fixture capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: destination,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
}

fn day(registry: &crate::Registry, state: &mut crate::AppState) {
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(registry, state).unwrap();
    apply_due_political_campaigns(registry, state);
}

#[test]
fn local_initiatives_require_real_venues_pay_multiple_accounts_and_observe_cooldowns() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let district = city.home;
    assert!(initiative_available(
        &city.state,
        city.player,
        district,
        CivicInitiative::PublicMeetings
    ));
    assert!(!initiative_available(
        &city.state,
        city.player,
        district,
        CivicInitiative::LocalPress
    ));
    fund(&mut city.state, city.player, 100_000);
    fund(&mut city.state, city.player, 100_000);
    fund(&mut city.state, city.player, 5_000_000);
    let manager = city
        .state
        .world()
        .characters_in_organization(city.player)
        .find(|person| {
            person.supervisor().is_some()
                && city
                    .state
                    .delegation()
                    .active_for_manager(person.id())
                    .is_some()
        })
        .unwrap()
        .id();
    let mandate = city.state.delegation().active_for_manager(manager).unwrap();
    let mut scopes = mandate.scopes().clone();
    scopes.insert(ResponsibilityScope::Neighborhood(district));
    scopes.insert(ResponsibilityScope::Function(
        ResponsibilityFunction::Political,
    ));
    validate_revise_mandate(
        &city.state,
        mandate.id(),
        MandateRevisionDraft {
            scopes,
            standing_orders: mandate.standing_orders().clone(),
            budget: mandate.budget(),
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    if city
        .state
        .world()
        .get_character(manager)
        .unwrap()
        .autonomy()
        != AutonomyLevel::Delegated
    {
        crate::world::world_system::validate_set_character_autonomy(
            &city.state,
            city.player,
            manager,
            AutonomyLevel::Delegated,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    let unchanged = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_civic_initiative(&city.state, district, manager, CivicInitiative::LocalPress)
            .err(),
        Some(PoliticalError::MissingCivicInfrastructure)
    );
    assert_eq!(
        unchanged,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );

    // Even at the machine-influence cap, the campaign must be able to
    // organize actual residents and institutions instead of hitting a dead end.
    political_system::validate_invest_influence(&city.state, district, 5_000_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_civic_initiative(
        &city.state,
        district,
        manager,
        CivicInitiative::PublicMeetings,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let office = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .district(district)
        .unwrap();
    assert_eq!(
        (
            office.influence(),
            office.grassroots(),
            office.institutions()
        ),
        (1_000, 70, 0)
    );
    assert_eq!(
        validate_civic_initiative(
            &city.state,
            district,
            manager,
            CivicInitiative::PublicMeetings
        )
        .err(),
        Some(PoliticalError::InitiativeCooldown)
    );
    let municipal = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let paid: i64 = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(municipal))
        .filter(|account| account.kind() == AccountKind::Settlement)
        .map(|account| account.balance().cents())
        .sum();
    assert_eq!(
        paid, 5_150_000,
        "a political expense must leave the criminal organization's books"
    );
    for _ in 0..7 {
        day(&registry, &mut city.state);
    }
    let unchanged = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_civic_initiative(
            &city.state,
            district,
            manager,
            CivicInitiative::PublicMeetings
        )
        .err(),
        Some(PoliticalError::InsufficientFunds)
    );
    assert_eq!(
        unchanged,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    fund(&mut city.state, city.player, 100_000);
    validate_civic_initiative(
        &city.state,
        district,
        manager,
        CivicInitiative::PublicMeetings,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn nomination_requires_a_real_sponsor_and_elections_record_loss_with_deterministic_continuation() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    fund(&mut city.state, city.player, 5_000_000);
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::IneligibleCandidate)
    );
    validate_establish_contact(
        &city.state,
        InstitutionalContactDraft {
            sponsor: city.player,
            handler: city.boss,
            contact: official,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    for _ in 0..crate::core::campaign_victory::POLITICAL_CONTACT_DAYS {
        day(&registry, &mut city.state);
    }
    use crate::core::campaign_victory::progress;
    use crate::social::RelationshipDimensions;
    use crate::social::relationship_system::validate_set_relationship;

    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .established_political_contacts,
        1
    );
    let held_nomination = validate_nominate_mayor(&city.state, city.boss).unwrap();
    let edges = [(city.boss, official), (official, city.boss)];
    let previous: Vec<_> = edges
        .into_iter()
        .filter_map(|(from, to)| {
            city.state
                .social()
                .get_relationship(from, to)
                .map(|record| (from, to, record.dimensions()))
        })
        .collect();
    for &(from, to, dimensions) in &previous {
        let dormant = RelationshipDimensions::zero();
        if dimensions != dormant {
            validate_set_relationship(&city.state, from, to, dormant)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
    }
    let dormant_progress = progress(&registry, &city.state).unwrap();
    assert_eq!(
        dormant_progress.active_political_contacts, 0,
        "an active historical record is not a usable sponsor"
    );
    assert_eq!(dormant_progress.established_political_contacts, 0);
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::IneligibleCandidate)
    );
    assert_eq!(
        held_nomination.commit(&mut city.state).err(),
        Some(PoliticalError::Stale)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    city.state = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .established_political_contacts,
        0
    );
    for (from, to, dimensions) in previous {
        if dimensions != RelationshipDimensions::zero() {
            validate_set_relationship(&city.state, from, to, dimensions)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
    }
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .established_political_contacts,
        1
    );
    // A rival claim to executive authority remains a split command even
    // while the second executive is temporarily detained.
    {
        use crate::core::entity::EntityRef;
        use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
        use crate::legal::investigation_system::{
            validate_add_evidence, validate_open_investigation,
        };
        use crate::legal::{
            Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
            EvidenceStrength, InvestigationDraft,
        };
        use crate::world::CharacterDraft;
        use crate::world::world_system::{insert_character, validate_set_character_supervisor};
        use std::collections::BTreeSet;

        let mut divided = city.state.clone();
        let held = validate_nominate_mayor(&divided, city.boss).unwrap();
        let rival_executive = insert_character(
            &mut divided,
            CharacterDraft {
                name: "Second claimant to leadership".into(),
                organization: Some(city.player),
                supervisor: None,
                autonomy: AutonomyLevel::Tight,
                capabilities: Default::default(),
                traits: Default::default(),
                drives: Default::default(),
            },
        )
        .unwrap();
        assert!(
            !progress(&registry, &divided)
                .unwrap()
                .mayoral_leader_eligible
        );
        let police = divided
            .world()
            .organizations()
            .find(|org| org.kind() == OrganizationKind::LawEnforcement)
            .unwrap()
            .id();
        let investigation = validate_open_investigation(
            &divided,
            InvestigationDraft {
                owner: police,
                title: "Second claimant's custody".into(),
                subjects: BTreeSet::from([EntityRef::Character(rival_executive)]),
            },
        )
        .unwrap()
        .commit(&mut divided)
        .unwrap();
        let mut evidence = BTreeSet::new();
        for kind in [EvidenceKind::Document, EvidenceKind::KnownAssociation] {
            evidence.insert(
                validate_add_evidence(
                    &divided,
                    EvidenceDraft {
                        investigation,
                        custodian: police,
                        subject: EntityRef::Character(rival_executive),
                        origin: None,
                        kind,
                        strength: EvidenceStrength::Strong,
                        reliability: EvidenceReliability::HighlyReliable,
                        admissibility: Admissibility::Admissible,
                        discovered_at: divided.now(),
                    },
                )
                .unwrap()
                .commit(&mut divided)
                .unwrap(),
            );
        }
        let arrest = validate_arrest(
            &registry,
            &divided,
            ArrestDraft {
                character: rival_executive,
                investigation,
                evidence,
            },
        )
        .unwrap()
        .commit(&mut divided)
        .unwrap();
        assert!(
            !progress(&registry, &divided)
                .unwrap()
                .mayoral_leader_eligible,
            "one free executive plus a detained second root is not a unified leadership"
        );
        assert_eq!(
            validate_nominate_mayor(&divided, city.boss).err(),
            Some(PoliticalError::IneligibleCandidate),
        );
        let before = bincode::serialize(&build_save(&registry, &divided).unwrap()).unwrap();
        assert_eq!(held.commit(&mut divided).err(), Some(PoliticalError::Stale));
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &divided).unwrap()).unwrap()
        );
        let mut restored =
            restore_save(&registry, build_save(&registry, &divided).unwrap()).unwrap();
        for state in [&mut divided, &mut restored] {
            validate_release_arrest(state, arrest)
                .unwrap()
                .commit(state)
                .unwrap();
            validate_set_character_supervisor(state, rival_executive, Some(city.boss))
                .unwrap()
                .commit(state)
                .unwrap();
            assert!(progress(&registry, state).unwrap().mayoral_leader_eligible);
            validate_nominate_mayor(state, city.boss)
                .unwrap()
                .commit(state)
                .unwrap();
            validate_invariants(state);
        }
        assert_eq!(
            bincode::serialize(&divided).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
    }
    // The exact filing cutoff is inclusive, and missing it by one minute
    // files for the following ballot, not the nearest calendar election.
    for (late_minutes, expected_day) in [(0, 360), (1, 540)] {
        let mut filing = city.state.clone();
        filing.advance_clock(SimDuration::from_minutes(
            (150 * DAY_MINUTES) as u32 + late_minutes,
        ));
        assert_eq!(
            political_system::next_municipal_election_day(filing.now()),
            Some(360)
        );
        assert_eq!(
            political_system::next_mayoral_nomination_day(filing.now()),
            Some(expected_day)
        );
        validate_nominate_mayor(&filing, city.boss)
            .unwrap()
            .commit(&mut filing)
            .unwrap();
        assert_eq!(
            filing
                .politics()
                .organization(city.player)
                .unwrap()
                .candidacy()
                .unwrap()
                .election_day,
            expected_day
        );
        let restored = restore_save(&registry, build_save(&registry, &filing).unwrap()).unwrap();
        assert_eq!(restored.politics(), filing.politics());
    }
    validate_nominate_mayor(&city.state, city.boss)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::AlreadyNominated)
    );
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .unwrap()
            .election_day,
        360
    );
    // A filed nomination cannot silently skip a reachable ballot. Forge
    // only the serialized date on a genuine paid filing and require the
    // release-safe loader to reject the otherwise plausible future ballot.
    let candidacy = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .candidacy()
        .unwrap();
    let envelope = build_save(&registry, &city.state).unwrap();
    let original = bincode::serialize(&candidacy).unwrap();
    let mut bytes = bincode::serialize(&envelope).unwrap();
    let positions: Vec<_> = bytes
        .windows(original.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    let delayed = MayoralCandidacy {
        election_day: candidacy.election_day + MUNICIPAL_ELECTION_INTERVAL_DAYS,
        ..candidacy
    };
    bytes.splice(
        positions[0]..positions[0] + original.len(),
        bincode::serialize(&delayed).unwrap(),
    );
    let forged = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(
        restore_save(&registry, forged),
        Err(crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPoliticalCampaign
        ))
    ));
    restore_save(&registry, envelope).unwrap();
    for _ in 180..359 {
        day(&registry, &mut city.state);
    }
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    day(&registry, &mut city.state);
    day(&registry, &mut restored);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    let election = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .latest_election()
        .unwrap();
    assert_eq!(election.election_day, 360);
    assert_eq!(election.candidate, city.boss);
    assert!(!election.elected);
    assert_eq!(election.wards_won, 0);
    assert_eq!(
        election.ballots.len(),
        city.state.world().neighborhoods().count()
    );
    assert!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .is_none()
    );
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    validate_nominate_mayor(&city.state, city.boss)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .candidacy()
            .unwrap()
            .election_day,
        540
    );
    validate_invariants(&city.state);
}

#[test]
fn former_nominee_joining_the_council_cannot_face_themself_on_the_ballot() {
    use crate::world::world_system::{
        insert_character, validate_reassign_character, validate_set_character_supervisor,
    };
    use crate::world::{CapabilityKind, CharacterDraft, Rating};
    use std::collections::BTreeMap;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    validate_establish_contact(
        &city.state,
        InstitutionalContactDraft {
            sponsor: city.player,
            handler: city.boss,
            contact: official,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    fund(&mut city.state, city.player, 5_000_000);
    for _ in 0..crate::core::campaign_victory::POLITICAL_CONTACT_DAYS {
        day(&registry, &mut city.state);
    }
    let nominee = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "New executive".into(),
            organization: Some(city.player),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([
                (CapabilityKind::Negotiation, Rating::try_new(100).unwrap()),
                (CapabilityKind::SocialAccess, Rating::try_new(100).unwrap()),
            ]),
            traits: Default::default(),
            drives: Default::default(),
        },
    )
    .unwrap();
    validate_set_character_supervisor(&city.state, city.boss, Some(nominee))
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_nominate_mayor(&city.state, nominee)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_set_character_supervisor(&city.state, city.boss, None)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_reassign_character(&city.state, nominee, Some(council), None)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();

    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for state in [&mut city.state, &mut restored] {
        state.advance_clock(SimDuration::from_minutes(
            (MUNICIPAL_ELECTION_INTERVAL_DAYS * DAY_MINUTES) as u32,
        ));
        apply_due_political_campaigns(&registry, state);
        let election = state
            .politics()
            .organization(city.player)
            .unwrap()
            .latest_election()
            .unwrap();
        assert_eq!(election.candidate, nominee);
        assert_ne!(
            election.incumbent,
            Some(nominee),
            "a candidate and incumbent must be distinct people"
        );
        assert!(!election.eligible);
        assert!(!election.elected);
        validate_invariants(state);
        restore_save(&registry, build_save(&registry, state).unwrap()).unwrap();
    }
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn mayoral_nomination_rejects_an_election_beyond_the_simulation_clock() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    validate_establish_contact(
        &city.state,
        InstitutionalContactDraft {
            sponsor: city.player,
            handler: city.boss,
            contact: official,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    use crate::core::time::SimTime;
    let final_ballot = (u64::MAX / DAY_MINUTES / MUNICIPAL_ELECTION_INTERVAL_DAYS)
        * MUNICIPAL_ELECTION_INTERVAL_DAYS;
    let final_cutoff = final_ballot * DAY_MINUTES - MAYORAL_FILING_LEAD_DAYS * DAY_MINUTES;
    assert_eq!(
        political_system::next_mayoral_nomination_day(SimTime::from_minutes(final_cutoff)),
        Some(final_ballot)
    );
    city.state
        .set_now_for_test(SimTime::from_minutes(final_cutoff + 1));
    let before = bincode::serialize(&city.state).unwrap();
    assert_eq!(
        political_system::next_municipal_election_day(city.state.now()),
        Some(final_ballot)
    );
    assert_eq!(
        political_system::next_mayoral_nomination_day(city.state.now()),
        None
    );
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::ElectionUnavailable)
    );
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);

    city.state.set_now_for_test(SimTime::MAX);
    let before = bincode::serialize(&city.state).unwrap();
    assert_eq!(
        political_system::next_municipal_election_day(city.state.now()),
        None
    );
    assert_eq!(
        validate_nominate_mayor(&city.state, city.boss).err(),
        Some(PoliticalError::ElectionUnavailable)
    );
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);
}
