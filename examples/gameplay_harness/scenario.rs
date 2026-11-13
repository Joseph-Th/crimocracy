//! Canonical authored opening built through production operations.
//!
//! The campaign is assembled from the same validated drafts, commits, and
//! authored content the shipped game uses. It exists so a player session can
//! start from a known, reproducible situation; it never selects commands or
//! interprets outcomes.

//! Scenario authoring: fixture construction, operation authorization helpers, and authored timeline derivation.

use crimocracy::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::state::AppState;
use crimocracy::core::time::DAY_MINUTES_U16;
use crimocracy::delegation::delegation_system::validate_assign_mandate;
use crimocracy::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crimocracy::economy::BusinessEconomyDraft;
use crimocracy::economy::business_economy_system::validate_establish_business_economy;
use crimocracy::enterprises::enterprise_execution::validate_establish_enterprise;
use crimocracy::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::intelligence_system::validate_record_information;
use crimocracy::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crimocracy::legal::jurisdiction_system::validate_set_jurisdiction;
use crimocracy::legal::patrol_system::validate_establish_patrol_deployment;
use crimocracy::legal::{DayMinute, JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow};
use crimocracy::registry::Registry;
use crimocracy::social::RelationshipDimensions;
use crimocracy::social::relationship_system::validate_set_relationship;
use crimocracy::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crimocracy::world::{
    ApprovalPolicy, AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
    CapabilityKind, CharacterDraft, DriveKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    PolicyKind, PolicySetting, TraitKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use crate::campaign::*;

pub fn build_scenario(
    registry: &Registry,
    seeds: EvaluationSeeds,
    profile: ScenarioProfile,
) -> Result<Scenario<'_>, Box<dyn Error>> {
    let mut state = AppState::new(seeds.world);
    let variation = FixtureVariation::from_seed(seeds.world);
    let timeline = ScenarioTimeline::for_profile(registry, seeds.policy, profile);

    let player = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Marrow Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let rival = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Rosetti Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let second_rival = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "D'Amato Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let police = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Central Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )?;
    let detective = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harlan Pike".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    designate_player_organization(&mut state, player)?;

    // Slight world-seed-derived jitter keeps the harness from testing one exact environment every
    // run while preserving deterministic matched-world comparisons.
    let jitter_rating = ((seeds.world >> 4) % 11) as i16 - 5; // -5..+5
    let jitter_minutes = ((seeds.world % 7) as i16 - 3) * 5; // -15..+15 in 5m steps
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: variation.neighborhood_name().to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(jitter_rating_u8(
                        variation.neighborhood_economy().0,
                        jitter_rating,
                    )),
                    commercial_activity: rating(jitter_rating_u8(
                        variation.neighborhood_economy().1,
                        jitter_rating,
                    )),
                    illicit_demand: rating(jitter_rating_u8(
                        variation.neighborhood_economy().2,
                        jitter_rating,
                    )),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(jitter_rating_u8(
                        profile.neighborhood_police_presence(variation),
                        jitter_rating,
                    )),
                },
            },
        },
    )?;
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(85),
        },
    )?
    .commit(&mut state)?;
    let patrol_windows = variation
        .patrol_windows(profile)
        .into_iter()
        .map(|(start, duration, presence)| {
            let jittered_start = (i32::from(start) + i32::from(jitter_minutes))
                .clamp(0, i32::from(DAY_MINUTES_U16) - i32::from(duration))
                as u16;
            Ok(PatrolWindow::try_new(
                DayMinute::try_new(jittered_start)?,
                duration,
                rating(jitter_rating_u8(presence, jitter_rating)),
            )?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: patrol_windows,
        },
    )?
    .commit(&mut state)?;

    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Joseph Marrow".to_owned(),
            organization: Some(player),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::from([
                (CapabilityKind::Management, rating(88)),
                (CapabilityKind::Negotiation, rating(75)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let lieutenant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Carlo Venn".to_owned(),
            organization: Some(player),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    rating(profile.lieutenant_management()),
                ),
                (CapabilityKind::Intimidation, rating(73)),
            ]),
            traits: BTreeSet::from([TraitKind::Ambitious, TraitKind::Secretive]),
            drives: BTreeMap::from([(DriveKind::Status, rating(78))]),
        },
    )?;
    let burglar = insert_character(
        &mut state,
        CharacterDraft {
            name: "Frank Dello".to_owned(),
            organization: Some(player),
            supervisor: Some(lieutenant),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Burglary, rating(profile.burglar_burglary())),
                (CapabilityKind::Stealth, rating(profile.burglar_stealth())),
                // The entry specialist doubles as the getaway driver so driver-role
                // operations (kidnapping, extraction) have a competent crew member.
                // Burglary itself never uses this capability; the repeal-pivot probe
                // was staffing an unskilled driver and teaching that new operations fail.
                (CapabilityKind::Driving, rating(65)),
            ]),
            traits: BTreeSet::from([TraitKind::EasilyFrightened]),
            drives: BTreeMap::from([(DriveKind::Safety, rating(88))]),
        },
    )?;
    let scout = insert_character(
        &mut state,
        CharacterDraft {
            name: "Mara Vale".to_owned(),
            organization: Some(player),
            supervisor: Some(lieutenant),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Surveillance,
                    rating(profile.scout_surveillance()),
                ),
                (CapabilityKind::Stealth, rating(profile.scout_stealth())),
            ]),
            traits: BTreeSet::from([TraitKind::Cautious]),
            drives: BTreeMap::new(),
        },
    )?;
    let bartender = insert_character(
        &mut state,
        CharacterDraft {
            name: "Lena Orr".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::SocialAccess, rating(65))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    // Danny Ferro is the act-2 replacement candidate: an independent the organization would need
    // to court through the canonical executive recruitment path after losing a crew member. The
    // authored candidate and pitch are fixed fixture policy rather than selected by inspecting his
    // latent drives or traits; those private characteristics remain production scoring inputs.
    let danny_ferro = insert_character(
        &mut state,
        CharacterDraft {
            name: "Danny Ferro".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Burglary, rating(70)),
                (CapabilityKind::Stealth, rating(58)),
            ]),
            traits: BTreeSet::from([TraitKind::Greedy]),
            drives: BTreeMap::from([(DriveKind::Money, rating(80))]),
        },
    )?;
    let rival_recruiter = insert_character(
        &mut state,
        CharacterDraft {
            name: "Maria Rosetti".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(60))]),
            traits: BTreeSet::from([TraitKind::Cautious]),
            drives: BTreeMap::new(),
        },
    )?;
    let victor_damato = insert_character(
        &mut state,
        CharacterDraft {
            name: "Victor D'Amato".to_owned(),
            organization: Some(second_rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::from([TraitKind::Proud]),
            drives: BTreeMap::new(),
        },
    )?;

    validate_set_relationship(
        &state,
        burglar,
        rival_recruiter,
        RelationshipDimensions {
            trust: level(10),
            respect: level(15),
            fear: level(20),
            affection: level(5),
            dependence: level(0),
            resentment: level(8),
            debt: level(0),
        },
    )?
    .commit(&mut state)?;

    validate_set_relationship(
        &state,
        burglar,
        lieutenant,
        RelationshipDimensions {
            trust: level(95),
            respect: level(90),
            fear: level(5),
            affection: level(85),
            dependence: level(90),
            resentment: level(5),
            debt: level(20),
        },
    )?
    .commit(&mut state)?;

    // The burglar also carries an older personal bond to the boss himself. Its strength varies
    // across authored worlds: sometimes that history is enough to win him back, and in the
    // crowded fixture it has cooled enough that leadership must live with a refusal and rebuild.
    // Act-1 rival poaching never reads this edge (its factors use only candidate-to-recruiter and
    // candidate-to-incumbent), so the variation changes recovery rather than causing the exit.
    let boss_burglar_relationship = match variation {
        FixtureVariation::Crowded => RelationshipDimensions {
            trust: level(60),
            respect: level(55),
            fear: level(15),
            affection: level(45),
            dependence: level(0),
            resentment: level(0),
            debt: level(20),
        },
        FixtureVariation::Clockwork | FixtureVariation::Quiet => RelationshipDimensions {
            trust: level(85),
            respect: level(75),
            fear: level(10),
            affection: level(70),
            dependence: level(0),
            resentment: level(0),
            debt: level(50),
        },
    };
    validate_set_relationship(&state, burglar, boss, boss_burglar_relationship)?
        .commit(&mut state)?;

    // Danny's pitch has a long-standing personal relationship to Marrow to stand on, so the
    // relationship edge runs from candidate to recruiter and executive recruitment stays
    // canonical. Production scoring still considers Danny's own private motives after the
    // fixture-selected pitch is submitted; harness policy does not inspect them to choose it.
    validate_set_relationship(
        &state,
        danny_ferro,
        boss,
        RelationshipDimensions {
            trust: level(70),
            respect: level(60),
            fear: level(10),
            affection: level(60),
            dependence: level(20),
            resentment: level(0),
            debt: level(40),
        },
    )?
    .commit(&mut state)?;

    // The boss keeps an old friendship with the precinct's lead detective: the organization's
    // standing Police-channel institutional contact. It is world state every branch can use;
    // only the arcs that have a reason to ask actually do.
    validate_set_relationship(
        &state,
        boss,
        detective,
        RelationshipDimensions {
            trust: level(45),
            respect: level(35),
            fear: level(0),
            affection: level(25),
            dependence: level(5),
            resentment: level(0),
            debt: level(10),
        },
    )?
    .commit(&mut state)?;

    validate_assign_mandate(
        &state,
        MandateDraft {
            organization: rival,
            manager: rival_recruiter,
            scopes: BTreeSet::from([
                ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                // Governed territory: with a district scope and a Broad-autonomy manager,
                // the rival's delegated expansion pass has real authority to act on.
                ResponsibilityScope::Neighborhood(neighborhood),
            ]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
            )]),
            budget: None,
        },
    )?
    .commit(&mut state)?;
    // Second rival: Victor D'Amato governs the same home district with delegated
    // expansion authority so rival_home_enterprises reflects a contested underworld
    // rather than a single actor. Both rivals can now recruit and expand.
    validate_assign_mandate(
        &state,
        MandateDraft {
            organization: second_rival,
            manager: victor_damato,
            scopes: BTreeSet::from([
                ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                ResponsibilityScope::Neighborhood(neighborhood),
            ]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
            )]),
            budget: None,
        },
    )?
    .commit(&mut state)?;

    // The primary target's owner: a shopkeeper who lives over the store. A witnessed or
    // identifying exposure on a character-owned business names him as the case's witness
    // through canonical incident intake, so institutional interviews, testimony quality,
    // and the organization's witness-pressure counter-play all have a real person to act on.
    let target_owner = insert_character(
        &mut state,
        CharacterDraft {
            name: "Emil Voss".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::SocialAccess, rating(40))]),
            traits: BTreeSet::from([TraitKind::Proud]),
            drives: BTreeMap::from([(DriveKind::Safety, rating(70))]),
        },
    )?;

    let target = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.target_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            neighborhood,
            owner: BusinessOwner::Character(target_owner),
        },
    )?;
    let alternate_target = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.alternate_target_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )?;
    let front = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.front_name().to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(player),
        },
    )?;
    let resale_venue = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.resale_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::Warehousing,
                BusinessFunction::ResaleMarket,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(player),
        },
    )?;

    // Rival-held venue in the home district: the Rosetti organization's delegated expansion
    // pass can host venue rackets here once its daily cadence comes due.
    let rival_venue = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.rival_venue_name().to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(rival),
        },
    )?;
    // Second rival's home-district venue: Victor D'Amato's organization now has a
    // competing claim in the same district so both rivals can expand and poach.
    let _damato_venue = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: "North Wharf Tavern".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(second_rival),
        },
    )?;

    // Second-district expansion fixture: a quiet harbor neighborhood outside Central Precinct's
    // jurisdiction, with an independently owned social club able to host a second gambling
    // enterprise. No jurisdiction is authored here on purpose: with no case-intake authority
    // there is no district heat, which is exactly the diversification lesson the PRESS arc
    // proves. The club starts independent on purpose too: hosting a racket requires owning the
    // venue, so diversification must run through the canonical acquisition path (clean money
    // buys the club) before the delegated expansion can establish anything there.
    let expansion_neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Harbor District".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(jitter_rating_u8(52, jitter_rating)),
                    commercial_activity: rating(jitter_rating_u8(58, jitter_rating)),
                    illicit_demand: rating(jitter_rating_u8(66, jitter_rating)),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(jitter_rating_u8(28, jitter_rating)),
                },
            },
        },
    )?;
    let expansion_front = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: "Pier Nine Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood: expansion_neighborhood,
            owner: BusinessOwner::Independent,
        },
    )?;
    // Empire-ladder fixture: an independent racing-wire office in the home district. Owning
    // it is the only way the organization can ever run a bookmaking book - the wire is a
    // network dependency no amount of cash at the club substitutes for - so buying it is a
    // capability unlock, not just one more venue. Starts independent so the empire beat pays
    // the authored price through the same canonical acquisition path as the harbor club.
    let wire_front = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: "Continental Racing Wire".to_owned(),
            kind: BusinessKind::NewsService,
            functions: BTreeSet::from([
                BusinessFunction::ProfessionalRecords,
                BusinessFunction::RacingWire,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )?;
    let expansion_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
        },
    )?;
    let expansion_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )?;
    // Idle-cash lending book: a street-cash till and settlement counterparty for the
    // second home-district racket the PRESS stand-down opens once the file cools. The
    // accounts exist in every branch's fixture (like the expansion pair) so IDs stay
    // comparable across matched branches; only the stand-down policy ever uses them.
    let loan_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
        },
    )?;
    let loan_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )?;
    // Bookmaking book (empire ladder): the wire-service purchase's paired racket account.
    // The book runs at the club, but its money is its own street-cash till.
    let bookmaking_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
        },
    )?;
    let bookmaking_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )?;

    let business_operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(front),
            kind: AccountKind::LegitimateOperating,
        },
    )?;
    let business_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(front),
            kind: AccountKind::Settlement,
        },
    )?;
    validate_establish_business_economy(
        registry,
        &state,
        BusinessEconomyDraft {
            business: front,
            operating_account: business_operating,
            settlement_account: business_settlement,
        },
    )?
    .commit(&mut state)?;

    let enterprise_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: if seeds.world.is_multiple_of(2) {
                AccountKind::StreetCash
            } else {
                AccountKind::ConcealedCash
            },
        },
    )?;
    let enterprise_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )?;
    let liquidation_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
        },
    )?;
    let liquidation_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
        },
    )?;
    // Clean-money destination: laundering routes street cash into the organization's
    // accounted funds through an owned cash-intensive front's canonical finance path.
    let accounted_funds = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::AccountedFunds,
        },
    )?;
    // Rival treasury accounts: the expansion pass draws its operating cash and a free
    // settlement account from these through the same ownership checks a player command faces.
    let rival_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::StreetCash,
        },
    )?;
    let rival_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::Settlement,
        },
    )?;
    // Seed operating capital: the organization opens with a small general treasury so the
    // daily payroll pass is a live carrying cost from the first campaign day. The offsetting
    // posting sits in a settlement account the financial view deliberately ignores.
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Seed the family treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: liquidation_settlement,
                    amount: Money::from_cents(-STARTING_TREASURY_CENTS),
                },
                LedgerPosting {
                    account: liquidation_cash,
                    amount: Money::from_cents(STARTING_TREASURY_CENTS),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut state)?;
    // Rival treasury: Rosetti needs enough street cash to keep its own payroll and to
    // fund the delegated expansion pass without an artificial capital starvation that
    // makes the "governed rival world" contract trivially true.
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Seed Rosetti treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: rival_settlement,
                    amount: Money::from_cents(-60_000),
                },
                LedgerPosting {
                    account: rival_cash,
                    amount: Money::from_cents(60_000),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut state)?;
    // Second rival treasury: D'Amato Crew mirrors Rosetti so both home-district
    // rivals can expand and recruit without fabricated starvation.
    let damato_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(second_rival),
            kind: AccountKind::StreetCash,
        },
    )?;
    let damato_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(second_rival),
            kind: AccountKind::Settlement,
        },
    )?;
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Seed D'Amato treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: damato_settlement,
                    amount: Money::from_cents(-60_000),
                },
                LedgerPosting {
                    account: damato_cash,
                    amount: Money::from_cents(60_000),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut state)?;
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: player,
            manager: lieutenant,
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(neighborhood),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )?
    .commit(&mut state)?;
    let police_contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor: player,
            handler: boss,
            contact: detective,
        },
    )?
    .commit(&mut state)?;
    let enterprise = validate_establish_enterprise(
        registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: player,
            authority: MandateAuthority {
                mandate,
                manager: lieutenant,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Business(front),
            supporting_businesses: BTreeSet::new(),
            cash_account: enterprise_cash,
            settlement_account: enterprise_settlement,
        },
    )?
    .commit(&mut state)?;

    let opportunity_information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(player),
            source_kind: InformationSourceKind::StreetRumor,
            topic: InformationTopic::TargetSecurity,
            source_entity: Some(EntityRef::Character(bartender)),
            subject: EntityRef::Business(target),
            observed_at: state.now(),
            reliability: variation.source_reliability(),
            specificity: variation.source_specificity(),
            summary: variation.source_summary().to_owned(),
        },
    )?
    .commit(&mut state)
    .expect("opportunity source information fixture should commit");
    let alternate_opportunity_information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(player),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: Some(EntityRef::Character(bartender)),
            subject: EntityRef::Business(alternate_target),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: variation.alternate_source_summary().to_owned(),
        },
    )?
    .commit(&mut state)
    .expect("alternate opportunity source information fixture should commit");

    let scenario = Scenario {
        registry,
        state,
        player,
        rival,
        second_rival,
        police,
        neighborhood,
        target,
        alternate_target,
        front,
        resale_venue,
        liquidation_cash,
        liquidation_settlement,
        accounted_funds,
        boss,
        lieutenant,
        burglar,
        scout,
        target_owner,
        danny_ferro,
        detective,
        police_contact,
        opportunity_information,
        alternate_opportunity_information,
        enterprise,
        expansion_neighborhood,
        expansion_front,
        expansion_cash,
        expansion_settlement,
        loan_cash,
        loan_settlement,
        wire_front,
        bookmaking_cash,
        bookmaking_settlement,
        rival_venue,
        rival_cash,
        rival_settlement,
        lieutenant_mandate: mandate,
        investigation: None,
        variation,
        timeline,
        seeds,
        wait_slack_minutes: operation_wait_slack_minutes(registry),
    };
    validate_campaign_state(scenario.registry, &scenario.state)?;
    Ok(scenario)
}
