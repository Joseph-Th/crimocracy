//! Scenario authoring: fixture construction, operation authorization helpers, and authored timeline derivation.

use crimocracy::contacts::contact_system::{validate_establish_contact, InstitutionalContactDraft};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, CharacterId, InformationId, OperationId, OpportunityId};
use crimocracy::core::state::AppState;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::delegation::delegation_system::MandateRevisionDraft;
use crimocracy::delegation::delegation_system::{validate_assign_mandate, validate_revise_mandate};
use crimocracy::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crimocracy::economy::business_economy_system::validate_establish_business_economy;
use crimocracy::economy::BusinessEconomyDraft;
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
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::{
    OperationApproach, OperationContingency, OperationDraft, OperationKind, OperationObjective,
    RoleKind,
};
use crimocracy::opportunities::opportunity_system::validate_discover_operation_opportunity;
use crimocracy::opportunities::OperationOpportunityDraft;
use crimocracy::recruitment::recruitment_system::validate_recruitment_attempt;
use crimocracy::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crimocracy::registry::Registry;
use crimocracy::social::relationship_system::validate_set_relationship;
use crimocracy::social::RelationshipDimensions;
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

use crate::*;

pub fn build_scenario(
    registry: &Registry,
    seed: u64,
    profile: ScenarioProfile,
) -> Result<Scenario<'_>, Box<dyn Error>> {
    let mut state = AppState::new(seed);
    let variation = FixtureVariation::from_seed(seed);
    let timeline = ScenarioTimeline::for_scenario(registry, seed);

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
            traits: BTreeSet::from([TraitKind::Patient]),
            drives: BTreeMap::new(),
        },
    )?;
    designate_player_organization(&mut state, player)?;

    // Slight seed-derived jitter keeps the harness from testing one exact clock every run while
    // preserving deterministic matched-seed comparisons.
    let jitter_rating = ((seed >> 4) % 11) as i16 - 5; // -5..+5
    let jitter_minutes = ((seed % 7) as i16 - 3) * 5; // -15..+15 in 5m steps
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
                        variation.neighborhood_police_presence(),
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
                .clamp(0, 1_440 - i32::from(duration)) as u16;
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
            traits: BTreeSet::from([TraitKind::Patient]),
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
    // to court through the canonical executive recruitment path after losing a crew member. His
    // Gambling-independent career means Burglary 70 / Stealth 58, and he already carries a
    // pre-existing personal relationship to the boss that makes the pitch deterministic without
    // any RNG or hidden-state reads.
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
    insert_character(
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
    .commit(&mut state);

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
    .commit(&mut state);

    // Danny's pitch leverages a long-standing personal debt to Marrow, so the relationship edges
    // run from the candidate to the recruiter and the executive recruitment path stays canonical.
    // The margin calculation reads only this authored relationship plus the registry definitions.
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
    .commit(&mut state);

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
    .commit(&mut state);

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
            owner: BusinessOwner::Independent,
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
    // pass can host venue rackets here once its daily cadence comes due. The D'Amato Crew has
    // no governed territory, so it stays inert by contrast.
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

    // Second-district expansion fixture: a quiet harbor neighborhood outside Central Precinct's
    // jurisdiction, with a player-owned social club able to host a second gambling enterprise.
    // No jurisdiction is authored here on purpose: with no case-intake authority there is no
    // district heat, which is exactly the diversification lesson the PRESS arc proves.
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
            owner: BusinessOwner::Organization(player),
        },
    )?;
    let expansion_business_operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(expansion_front),
            kind: AccountKind::LegitimateOperating,
        },
    )?;
    let expansion_business_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(expansion_front),
            kind: AccountKind::Settlement,
        },
    )?;
    validate_establish_business_economy(
        registry,
        &state,
        BusinessEconomyDraft {
            business: expansion_front,
            operating_account: expansion_business_operating,
            settlement_account: expansion_business_settlement,
        },
    )?
    .commit(&mut state)?;
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

    let cash_kind = if seed.is_multiple_of(2) {
        AccountKind::StreetCash
    } else {
        AccountKind::ConcealedCash
    };
    let enterprise_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: cash_kind,
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
                    amount: Money::from_cents(-80_000),
                },
                LedgerPosting {
                    account: liquidation_cash,
                    amount: Money::from_cents(80_000),
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
        boss,
        lieutenant,
        burglar,
        scout,
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
        rival_venue,
        rival_cash,
        rival_settlement,
        lieutenant_mandate: mandate,
        investigation: None,
        variation,
        timeline,
    };
    validate_harness_state(scenario.registry, &scenario.state)?;
    Ok(scenario)
}

pub fn authorize_surveillance(scenario: &mut Scenario) -> Result<OperationId, Box<dyn Error>> {
    let title = format!("{} surveillance", scenario.variation.target_name());
    authorize_surveillance_target(
        scenario,
        EntityRef::Business(scenario.target),
        &title,
        scenario.state.now() + SimDuration::ONE_MINUTE,
    )
}

pub fn authorize_surveillance_target(
    scenario: &mut Scenario,
    target: EntityRef,
    title: &str,
    scheduled_for: SimTime,
) -> Result<OperationId, Box<dyn Error>> {
    Ok(validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: title.to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: scenario.player,
            leader: scenario.scout,
            objective: OperationObjective::GatherInformation { target },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, scenario.scout)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?)
}

/// Describes the surveillance plan level visible to the player from a discovered police-org
/// observation: active-case heat versus a shelved case. Returns `None` when no legal-activity
/// observation about the authority was produced.
pub fn observe_authority_case_sightline(
    scenario: &Scenario,
    resolution: &crimocracy::operations::OperationResolutionRecord,
) -> Option<bool> {
    resolution
        .discovered_information()
        .iter()
        .find_map(|information| {
            let record = scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("discovered surveillance information must persist");
            if record.topic() != InformationTopic::LegalActivity {
                return None;
            }
            observe_authority_case_sightline_summary(record.summary())
        })
}

/// Parses a player-visible case-activity summary into the sightline read: Some(true) means the
/// authority is still visibly developing the known case, Some(false) that it appears shelved.
/// Both counterintelligence channels (precinct surveillance and contact disclosure) phrase
/// their summaries with the canonical markers from `legal::case_knowledge`, so the acting
/// policy never needs hidden state.
pub fn observe_authority_case_sightline_summary(summary: &str) -> Option<bool> {
    crimocracy::legal::case_knowledge::CaseActivityStatus::parse_summary_marker(summary)
        .and_then(|status| status.is_hot())
}

/// Fixture-authored contact knowledge was deleted: the lead detective's case knowledge is now
/// production state, recorded when staffing assigns him the case and refreshed when cold-case
/// decay shelves or closes it (`legal::case_knowledge`). Contact channels read it through the
/// canonical pending-disclosure and disclosure paths.
pub fn authorize_burglary(
    scenario: &mut Scenario,
    strategy: Strategy,
    target: BusinessId,
    title: &str,
    scheduled_for: SimTime,
    intelligence: BTreeSet<InformationId>,
    entry_specialist: CharacterId,
) -> Result<OperationId, Box<dyn Error>> {
    let contingencies = match strategy {
        Strategy::Rush | Strategy::Recon => vec![
            OperationContingency::AbortOnPoliceArrivalBeforeEntry,
            OperationContingency::RequestDecisionOnUnexpectedCondition,
        ],
        Strategy::Press => vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
    };
    Ok(validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: title.to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.lieutenant),
                (RoleKind::EntrySpecialist, entry_specialist),
            ]),
            intelligence,
            constraints: Vec::new(),
            contingencies,
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?)
}

/// The narrative act-2 opening: at the canonical discovery minute every narrative branch sees the
/// alternate target's second score as a fresh player-visible opportunity, committed through the
/// canonical discovery path so conversion, expiry, and their reports all follow production rules.
pub fn discover_second_opportunity(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<OpportunityId, Box<dyn Error>> {
    let discovered_at = scenario.state.now();
    debug_assert!(
        discovered_at >= scenario.timeline.second_opportunity_discovery_at,
        "second opportunity discovery must not be authored earlier than its scenario timeline"
    );
    let valid_until = scenario.timeline.second_opportunity_valid_until;
    let opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.alternate_target)]),
            source_information: BTreeSet::from([scenario.alternate_opportunity_information]),
            summary: format!(
                "{} is moving high-value stock again; the second score on {} is available until the window closes.",
                scenario.variation.alternate_target_name(),
                scenario
                    .state
                    .world()
                    .get_neighborhood(scenario.neighborhood)
                    .expect("neighborhood must persist")
                    .name(),
            ),
            valid_until: Some(valid_until),
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.second_opportunity = Some(opportunity);
    metrics.second_opportunity_discovered = true;
    if narrative {
        let record = scenario
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("committed second opportunity must be queryable");
        println!("[OBSERVE] {}", stamp(discovered_at.as_minutes()));
        println!("          Opportunity: {}", record.summary());
        println!(
            "          Source: {}",
            scenario
                .state
                .intelligence()
                .get_information(scenario.alternate_opportunity_information)
                .expect("alternate source information must persist")
                .summary()
        );
        println!(
            "          The second score expires at {}.",
            format_minute_of_day(valid_until.as_minutes())
        );
    }
    Ok(opportunity)
}

/// The PRESS wait becomes governance instead of dead time: revise the lieutenant's mandate to
/// cover both districts, move idle street cash into a harbor float through a canonical ledger
/// transfer, and establish a second gambling enterprise delegated under the revised mandate.
/// Every step is a production path a player would drive from the delegation and finance views;
/// nothing here reads hidden case state, and the harbor district pays no heat surcharge because
/// Central Precinct's active case never touched it.
pub fn establish_harbor_expansion(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    const EXPANSION_FLOAT_CENTS: i64 = 40_000;
    let canal_cash = scenario
        .state
        .enterprises()
        .get_enterprise(scenario.enterprise)
        .expect("canal enterprise must persist")
        .cash_account();
    let neighborhood_name = scenario
        .state
        .world()
        .get_neighborhood(scenario.neighborhood)
        .expect("canal neighborhood must persist")
        .name()
        .to_owned();
    let lieutenant_name = scenario
        .state
        .world()
        .get_character(scenario.lieutenant)
        .expect("lieutenant must persist")
        .name()
        .to_owned();

    validate_revise_mandate(
        &scenario.state,
        scenario.lieutenant_mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(scenario.neighborhood),
                ResponsibilityScope::Neighborhood(scenario.expansion_neighborhood),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )?
    .commit(&mut scenario.state)?;
    validate_record_transaction(
        &scenario.state,
        LedgerTransactionDraft {
            occurred_at: scenario.state.now(),
            memo: "Capitalize the Harbor District book".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: canal_cash,
                    amount: Money::from_cents(-EXPANSION_FLOAT_CENTS),
                },
                LedgerPosting {
                    account: scenario.expansion_cash,
                    amount: Money::from_cents(EXPANSION_FLOAT_CENTS),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut scenario.state)?;
    let enterprise = validate_establish_enterprise(
        scenario.registry,
        &scenario.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: scenario.player,
            authority: MandateAuthority {
                mandate: scenario.lieutenant_mandate,
                manager: scenario.lieutenant,
                scope: ResponsibilityScope::Neighborhood(scenario.expansion_neighborhood),
            },
            location: EnterpriseLocation::Business(scenario.expansion_front),
            supporting_businesses: BTreeSet::new(),
            cash_account: scenario.expansion_cash,
            settlement_account: scenario.expansion_settlement,
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.expansion_enterprise = Some(enterprise);
    metrics.expansion_established = true;
    if narrative {
        println!(
            "[DECIDE]  Standing down does not mean standing still. Revise {lieutenant_name}'s mandate to cover both districts, capitalize a harbor float from idle gambling cash, and open a second book in Harbor District."
        );
        println!(
            "[DELEGATE] {} now holds an expanded two-district mandate (v2); routine authority over the new enterprise is delegated.",
            lieutenant_name
        );
        println!(
            "[EXPAND]   Gambling enterprise established at Pier Nine Social Club (Harbor District) with a {} float.",
            format_cents(EXPANSION_FLOAT_CENTS)
        );
        println!(
            "[NARRATION] Harbor District sits outside Central Precinct's jurisdiction: the open case that taxes the {neighborhood_name} racket cannot reach this one."
        );
    }
    Ok(())
}

/// The RUSH rebuild beat: after an autonomous rival departure removed the entry specialist, the
/// player works the canonical executive recruitment path to court the independent candidate. The
/// candidate relationship is authored so acceptance is deterministic and identical across seeds;
/// the recruitment decision itself never reads hidden or audit state.
pub fn recruit_replacement(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<CharacterId, Box<dyn Error>> {
    let candidate = scenario.danny_ferro;
    let recruiter = scenario.boss;
    let organization = scenario.player;
    let attempt = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            target_organization: organization,
            recruiter,
            candidate,
            approach: RecruitmentApproach::FinancialOpportunity,
        },
    )?
    .commit(&mut scenario.state)?;
    let attempt_record = scenario
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("committed executive recruitment must be queryable");
    if attempt_record.outcome() != crimocracy::recruitment::RecruitmentOutcome::Accepted {
        let candidate_name = scenario
            .state
            .world()
            .get_character(candidate)
            .expect("replacement candidate must persist")
            .name()
            .to_owned();
        return Err(format!(
            "replacement recruitment of {candidate_name} was {:?}; the rebuilt-crew contract requires acceptance",
            attempt_record.outcome()
        )
        .into());
    }
    let record = scenario
        .state
        .world()
        .get_character(candidate)
        .expect("recruited replacement must persist");
    if record.organization() != Some(organization) {
        return Err(
            "replacement recruitment committed without a player-organization membership".into(),
        );
    }
    metrics.replacement = Some(candidate);
    metrics.replacement_recruited = true;
    if narrative {
        let candidate_name = record.name();
        let recruiter_name = scenario
            .state
            .world()
            .get_character(recruiter)
            .expect("recruiter must persist")
            .name();
        let organization_name = scenario
            .state
            .world()
            .get_organization(organization)
            .expect("player organization must persist")
            .name();
        println!(
            "[DECIDE]  Leadership personally recruited {candidate_name}: Marrow made the {:?} pitch and {candidate_name} accepted, joining {organization_name} as the replacement entry specialist.",
            attempt_record.approach()
        );
        println!(
            "[NARRATION] {recruiter_name}'s pitch was backed by an existing relationship and the candidate's greed drive; margin {}, outcome {:?}. No hidden or audit state influenced the decision.",
            attempt_record.margin(),
            attempt_record.outcome()
        );
    }
    Ok(candidate)
}
