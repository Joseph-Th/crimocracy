//! Seeded generation of a complete *simulated* city, not a second city model.
//!
//! District profiles, businesses, staffing, criminal rivals, legal institutions,
//! jurisdictions, patrols, relations, operating economies, finances, rackets and
//! initial player leads are inserted through their normal production owners.
//! The generator has its own local random stream: it never consumes the
//! campaign's four persisted simulation RNG streams. CityScene can project
//! streets, buildings and lots from the resulting world with the same seed.

use crate::city_view::{CityScene, CityViewError};
use crate::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, CharacterId, FinancialAccountId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::patrol_system::validate_establish_patrol_deployment;
use crate::legal::{DayMinute, JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow};
use crate::operations::OperationKind;
use crate::opportunities::OperationOpportunityDraft;
use crate::opportunities::opportunity_system::validate_discover_operation_opportunity;
use crate::registry::Registry;
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crate::world::{
    ALL_BUSINESS_KINDS, ApprovalPolicy, AutonomyLevel, BusinessDraft, BusinessFunction as F,
    BusinessKind as K, BusinessOwner, CapabilityKind as C, CharacterDraft, DriveKind,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, PolicyKind, PolicySetting, Rating,
    TraitKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

pub type GenerationResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CityScale {
    Small,
    Medium,
    Large,
}

impl CityScale {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
    pub const fn config(self, seed: u64) -> CityGenerationConfig {
        match self {
            Self::Small => CityGenerationConfig {
                seed,
                districts: 4,
                businesses_per_district: 12,
                rivals: 2,
            },
            Self::Medium => CityGenerationConfig {
                seed,
                districts: 8,
                businesses_per_district: 18,
                rivals: 3,
            },
            Self::Large => CityGenerationConfig {
                seed,
                districts: 16,
                businesses_per_district: 24,
                rivals: 5,
            },
        }
    }
}

/// Bounded custom city shape. Business density stays below the CityScene lot
/// limit (1,024 per district), and districts exceed the two authored fixtures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CityGenerationConfig {
    pub seed: u64,
    pub districts: u8,
    pub businesses_per_district: u8,
    pub rivals: u8,
}

impl CityGenerationConfig {
    pub fn validate(self) -> GenerationResult<()> {
        if !(4..=24).contains(&self.districts)
            || !(8..=40).contains(&self.businesses_per_district)
            || !(1..=8).contains(&self.rivals)
            || u16::from(self.rivals) >= u16::from(self.districts)
        {
            return Err("city requires 4-24 districts, 8-40 businesses per district, and 1-8 rivals fewer than districts".into());
        }
        Ok(())
    }
}

/// Typed initial identities are for the campaign adapter only. They are not
/// part of the save format: restored worlds retain every identity directly.
pub struct GeneratedCity {
    pub state: AppState,
    pub player: OrganizationId,
    pub home: NeighborhoodId,
    pub boss: CharacterId,
    pub primary_target: BusinessId,
    pub secondary_target: BusinessId,
    pub starter_front: BusinessId,
    pub starter_racket: crate::core::id::EnterpriseId,
    pub police: Vec<OrganizationId>,
    pub rivals: Vec<OrganizationId>,
    pub seed: u64,
}

impl GeneratedCity {
    pub fn scene(&self) -> Result<CityScene, CityViewError> {
        CityScene::generate(self.state.world(), self.seed ^ 0x9E37_79B9_7F4A_7C15)
    }
}

/// SplitMix64 is a fixed, platform-independent construction stream. It is
/// intentionally separate from gameplay RNG and serialized game evolution.
struct CityRng {
    state: u64,
    used_names: BTreeSet<String>,
}
impl CityRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0xC17C_1A59_0D3E_C123,
            used_names: BTreeSet::new(),
        }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn range(&mut self, limit: usize) -> usize {
        // Multiplication-high gives an unbiased-enough uniform index for city
        // content without platform-specific usize overflow or modulo patterns.
        ((u128::from(self.next()) * limit as u128) >> 64) as usize
    }
    fn rating(&mut self, low: u8, high: u8) -> Rating {
        rating(low + self.range(usize::from(high - low) + 1) as u8)
    }
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("generator rating stays in 0..=100")
}
fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("generator relationship level stays in 0..=100")
}

const WARD_PREFIXES: &[&str] = &[
    "Amber", "Ash", "Briar", "Canal", "Cedar", "Cobalt", "Copper", "Crown", "East", "Elm",
    "Foundry", "Granite", "Harbor", "Iron", "Juniper", "Lantern", "Laurel", "Mercer", "Mill",
    "North", "Old", "Pier", "Red", "River", "Sable", "Silver", "South", "Station", "Stone",
    "Union", "West", "Willow",
];
const WARD_SUFFIXES: &[&str] = &[
    "Ward", "Quarter", "Heights", "Docks", "Yards", "Row", "Flats", "Point", "District", "Hill",
    "Crossing", "Market", "End", "Harbor", "Commons", "Basin",
];
const SURNAMES: &[&str] = &[
    "Venn",
    "Marrow",
    "Rosetti",
    "Kessler",
    "Doyle",
    "Caldwell",
    "Keene",
    "Orr",
    "Mercer",
    "Bell",
    "Silva",
    "Caruso",
    "Vale",
    "Dello",
    "Ferro",
    "Novak",
    "Moretti",
    "Hawthorne",
    "Brennan",
    "Alvarez",
    "Larkin",
    "DeLuca",
    "Serrano",
    "Crowe",
    "Oakes",
    "Finch",
    "Madden",
    "Voss",
    "Pike",
    "Marlow",
    "Ward",
    "Sloan",
];
const GIVEN_NAMES: &[&str] = &[
    "Ada", "Alex", "Amelia", "Arthur", "Beatrice", "Cal", "Carmen", "Clara", "Daniel", "Edith",
    "Elena", "Felix", "Frank", "Grace", "Hector", "Inez", "Iris", "Joseph", "June", "Leo", "Lena",
    "Marco", "Mara", "Nora", "Owen", "Pearl", "Rosa", "Sam", "Sofia", "Theo", "Vera", "Victor",
];
const SHOP_PREFIXES: &[&str] = &[
    "Adler",
    "Bell",
    "Bridge",
    "Calder",
    "Dover",
    "Ellis",
    "Fulton",
    "Grafton",
    "Hollis",
    "Keystone",
    "Linden",
    "Marlow",
    "Northline",
    "Parker",
    "Redline",
    "Sable",
    "Tanner",
    "Union",
    "Vesper",
    "Whitmore",
    "Arcadia",
    "Crown",
];

fn person_name(rng: &mut CityRng, _suffix: usize) -> String {
    loop {
        let name = format!(
            "{} {}",
            GIVEN_NAMES[rng.range(GIVEN_NAMES.len())],
            SURNAMES[rng.range(SURNAMES.len())],
        );
        if rng.used_names.insert(name.clone()) {
            return name;
        }
    }
}

fn relationship(trust: u8, respect: u8, affection: u8) -> RelationshipDimensions {
    RelationshipDimensions {
        trust: level(trust),
        respect: level(respect),
        fear: level(0),
        affection: level(affection),
        dependence: level(12),
        resentment: level(0),
        debt: level(5),
    }
}

fn add_character(
    state: &mut AppState,
    name: String,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    autonomy: AutonomyLevel,
    capabilities: &[(C, u8)],
    traits: &[TraitKind],
) -> GenerationResult<CharacterId> {
    Ok(insert_character(
        state,
        CharacterDraft {
            name,
            organization,
            supervisor,
            autonomy,
            capabilities: capabilities
                .iter()
                .map(|(kind, value)| (*kind, rating(*value)))
                .collect(),
            traits: traits.iter().copied().collect(),
            drives: BTreeMap::from([
                (DriveKind::Money, rating(48)),
                (DriveKind::Safety, rating(58)),
            ]),
        },
    )?)
}

fn add_account(
    state: &mut AppState,
    owner: FinancialOwner,
    kind: AccountKind,
) -> GenerationResult<FinancialAccountId> {
    Ok(insert_account(
        state,
        FinancialAccountDraft { owner, kind },
    )?)
}

fn treasury(
    state: &mut AppState,
    org: OrganizationId,
    cents: i64,
) -> GenerationResult<(FinancialAccountId, FinancialAccountId)> {
    let owner = FinancialOwner::Organization(org);
    let cash = add_account(state, owner, AccountKind::StreetCash)?;
    let settlement = add_account(state, owner, AccountKind::Settlement)?;
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Generated city starting treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: cash,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )?
    .commit(state)?;
    Ok((cash, settlement))
}

fn add_business(
    registry: &Registry,
    state: &mut AppState,
    name: String,
    kind: K,
    functions: &[F],
    district: NeighborhoodId,
    owner: BusinessOwner,
) -> GenerationResult<BusinessId> {
    let business = insert_business(
        registry,
        state,
        BusinessDraft {
            name,
            kind,
            functions: functions.iter().copied().collect(),
            neighborhood: district,
            owner,
        },
    )?;
    // Every real venue gets an independent operating ledger and a settlement
    // clock. An independent shop is not just scenery until it is acquired.
    let operating = add_account(
        state,
        FinancialOwner::Business(business),
        AccountKind::LegitimateOperating,
    )?;
    let settlement = add_account(
        state,
        FinancialOwner::Business(business),
        AccountKind::Settlement,
    )?;
    validate_establish_business_economy(
        registry,
        state,
        BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )?
    .commit(state)?;
    Ok(business)
}

fn business_template(kind: K) -> (&'static str, &'static [F]) {
    use F::*;
    match kind {
        K::Retail => ("Stores", &[CustomerAccess, ProfessionalRecords]),
        K::Hospitality => (
            "Social Club",
            &[CashIntensive, MeetingSpace, CustomerAccess],
        ),
        K::Automotive => ("Auto Works", &[VehicleWorkshop, CustomerAccess]),
        K::Transportation => ("Freight", &[VehicleFleet, DistributionInfrastructure]),
        K::Warehouse => ("Storage", &[Warehousing, DistributionInfrastructure]),
        K::ProfessionalServices => ("Accounting", &[ProfessionalRecords, CustomerAccess]),
        K::Brewery => ("Brewery", &[AlcoholProduction, Warehousing]),
        K::Nightclub => ("Nightclub", &[CashIntensive, Nightlife, MeetingSpace]),
        K::Construction => ("Builders", &[VehicleFleet, Warehousing]),
        K::Wholesale => ("Wholesale", &[Warehousing, DistributionInfrastructure]),
        K::Pawnshop => (
            "Exchange",
            &[CashIntensive, ResaleMarket, CustomerAccess, Warehousing],
        ),
        K::CoinMachineDistribution => (
            "Coin Machines",
            &[DistributionInfrastructure, CashIntensive],
        ),
        K::Lodging => ("Hotel", &[Lodging, CustomerAccess]),
        K::AthleticClub => ("Gymnasium", &[SportingVenue, MeetingSpace]),
        K::Laundry => ("Laundry", &[CashIntensive, CustomerAccess]),
        K::Printing => ("Printing", &[PrintingPress, ProfessionalRecords]),
        K::FinancialServices => ("Savings", &[FinancialServices, ProfessionalRecords]),
        K::GarmentFactory => ("Textiles", &[Warehousing, DistributionInfrastructure]),
        K::Stevedoring => ("Dock Services", &[DockAccess, Warehousing]),
        K::NewsService => ("Racing Wire", &[RacingWire, ProfessionalRecords]),
        K::Pharmacy => ("Pharmacy", &[CustomerAccess, ProfessionalRecords]),
        K::FuneralParlor => ("Funeral Home", &[MeetingSpace, CustomerAccess]),
        K::SoupKitchen => ("Kitchen", &[CustomerAccess, CashIntensive]),
        K::TaxiGarage => ("Taxi Garage", &[VehicleFleet, VehicleWorkshop]),
        K::MovieTheater => ("Picture House", &[CustomerAccess, CashIntensive]),
    }
}

/// Construct a new, playable city from a seed and bounded structural budget.
/// No scenario fixture IDs, policy scripts, or hidden outcome injection are used.
pub fn generate_city(
    registry: &Registry,
    config: CityGenerationConfig,
) -> GenerationResult<GeneratedCity> {
    config.validate()?;
    let mut rng = CityRng::new(config.seed);
    let mut state = AppState::new(config.seed);
    let player = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: format!("{} Organization", SURNAMES[rng.range(SURNAMES.len())]),
            kind: OrganizationKind::Criminal,
        },
    )?;
    designate_player_organization(&mut state, player)?;
    let rivals = (0..usize::from(config.rivals))
        .map(|index| {
            insert_organization(
                registry,
                &mut state,
                OrganizationDraft {
                    name: format!("{} Crew {}", SURNAMES[rng.range(SURNAMES.len())], index + 1),
                    kind: OrganizationKind::Criminal,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let precinct_count = usize::from(config.districts).div_ceil(4);
    let police = (0..precinct_count)
        .map(|index| {
            insert_organization(
                registry,
                &mut state,
                OrganizationDraft {
                    name: format!("Precinct {}", index + 1),
                    kind: OrganizationKind::LawEnforcement,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let legal_services = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Municipal Defense Office".to_owned(),
            kind: OrganizationKind::LegalServices,
        },
    )?;
    let prosecutor = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "City Prosecution Bureau".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )?;
    let council = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "City Council".to_owned(),
            kind: OrganizationKind::Political,
        },
    )?;
    let labor = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "City Labor Federation".to_owned(),
            kind: OrganizationKind::Labor,
        },
    )?;
    let commerce = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Merchants Association".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )?;

    let mut ward_names = BTreeSet::new();
    let mut districts = Vec::new();
    for index in 0..usize::from(config.districts) {
        let name = loop {
            let candidate = format!(
                "{} {}",
                WARD_PREFIXES[rng.range(WARD_PREFIXES.len())],
                WARD_SUFFIXES[rng.range(WARD_SUFFIXES.len())]
            );
            if ward_names.insert(candidate.clone()) {
                break candidate;
            }
        };
        // A seeded archetype creates correlated economics and policing rather
        // than independently rolling every rating into a placeless gray city.
        let archetype = (index + rng.range(6)) % 6;
        let (wealth, commerce, demand, policing) = match archetype {
            0 => ((72, 93), (78, 98), (57, 83), (55, 86)), // business center
            1 => ((28, 50), (42, 69), (66, 92), (29, 59)), // industrial
            2 => ((36, 65), (60, 85), (69, 95), (38, 72)), // waterfront
            3 => ((55, 82), (53, 76), (41, 69), (48, 79)), // residential
            4 => ((61, 85), (70, 97), (75, 98), (57, 87)), // nightlife
            _ => ((24, 56), (34, 63), (53, 85), (19, 52)), // fringe
        };
        let id = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name,
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rng.rating(wealth.0, wealth.1),
                        commercial_activity: rng.rating(commerce.0, commerce.1),
                        illicit_demand: rng.rating(demand.0, demand.1),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rng.rating(policing.0, policing.1),
                    },
                },
            },
        )?;
        districts.push(id);
    }
    let home = districts[0];

    // Every neighborhood has explicit police case-intake routing and a real
    // two-window patrol. Precinct territories partition the whole city.
    for (index, precinct) in police.iter().copied().enumerate() {
        let covered = districts
            .iter()
            .copied()
            .enumerate()
            .filter(|(ward, _)| ward % precinct_count == index)
            .map(|(_, district)| district)
            .collect::<BTreeSet<_>>();
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: precinct,
                neighborhoods: covered.clone(),
                case_intake_priority: rating(75 + rng.range(20) as u8),
            },
        )?
        .commit(&mut state)?;
        for district in covered {
            let first = 80 + rng.range(250) as u16;
            let second = 1_040 + rng.range(230) as u16;
            validate_establish_patrol_deployment(
                &state,
                PatrolDeploymentDraft {
                    organization: precinct,
                    neighborhood: district,
                    windows: vec![
                        PatrolWindow::try_new(DayMinute::try_new(first)?, 120, rng.rating(58, 96))?,
                        PatrolWindow::try_new(
                            DayMinute::try_new(second)?,
                            120,
                            rng.rating(36, 80),
                        )?,
                    ],
                },
            )?
            .commit(&mut state)?;
        }
    }

    let mut detectives = Vec::new();
    for (index, precinct) in police.iter().copied().enumerate() {
        let detective = add_character(
            &mut state,
            person_name(&mut rng, index),
            Some(precinct),
            None,
            AutonomyLevel::Delegated,
            &[(C::Investigation, 83), (C::Management, 66)],
            &[],
        )?;
        add_character(
            &mut state,
            person_name(&mut rng, 50 + index),
            Some(precinct),
            Some(detective),
            AutonomyLevel::Guided,
            &[(C::Investigation, 66), (C::Violence, 52)],
            &[],
        )?;
        detectives.push(detective);
    }
    let lawyer = add_character(
        &mut state,
        person_name(&mut rng, 100),
        Some(legal_services),
        None,
        AutonomyLevel::Delegated,
        &[(C::LegalKnowledge, 91), (C::Negotiation, 72)],
        &[],
    )?;
    add_account(
        &mut state,
        FinancialOwner::Organization(legal_services),
        AccountKind::LegitimateOperating,
    )?;
    add_character(
        &mut state,
        person_name(&mut rng, 101),
        Some(prosecutor),
        None,
        AutonomyLevel::Broad,
        &[(C::LegalKnowledge, 86), (C::Investigation, 78)],
        &[],
    )?;
    let councilor = add_character(
        &mut state,
        person_name(&mut rng, 102),
        Some(council),
        None,
        AutonomyLevel::Broad,
        &[(C::Negotiation, 82), (C::SocialAccess, 77)],
        &[],
    )?;
    let union_delegate = add_character(
        &mut state,
        person_name(&mut rng, 103),
        Some(labor),
        None,
        AutonomyLevel::Broad,
        &[(C::Management, 72), (C::SocialAccess, 79)],
        &[],
    )?;
    let merchant = add_character(
        &mut state,
        person_name(&mut rng, 104),
        Some(commerce),
        None,
        AutonomyLevel::Broad,
        &[(C::Negotiation, 79), (C::SocialAccess, 83)],
        &[],
    )?;

    let boss = add_character(
        &mut state,
        person_name(&mut rng, 200),
        Some(player),
        None,
        AutonomyLevel::Tight,
        &[(C::Management, 88), (C::Negotiation, 79)],
        &[],
    )?;
    let lieutenant = add_character(
        &mut state,
        person_name(&mut rng, 201),
        Some(player),
        Some(boss),
        AutonomyLevel::Delegated,
        &[(C::Management, 79), (C::Intimidation, 77)],
        &[TraitKind::Ambitious],
    )?;
    let enforcer = add_character(
        &mut state,
        person_name(&mut rng, 202),
        Some(player),
        Some(lieutenant),
        AutonomyLevel::Guided,
        &[(C::Violence, 83), (C::Intimidation, 69)],
        &[],
    )?;
    let burglar = add_character(
        &mut state,
        person_name(&mut rng, 203),
        Some(player),
        Some(lieutenant),
        AutonomyLevel::Guided,
        &[(C::Burglary, 86), (C::Stealth, 78), (C::Driving, 68)],
        &[],
    )?;
    let scout = add_character(
        &mut state,
        person_name(&mut rng, 204),
        Some(player),
        Some(lieutenant),
        AutonomyLevel::Guided,
        &[(C::Surveillance, 91), (C::Stealth, 81)],
        &[TraitKind::Cautious],
    )?;
    add_character(
        &mut state,
        person_name(&mut rng, 205),
        Some(player),
        Some(lieutenant),
        AutonomyLevel::Guided,
        &[(C::Driving, 91), (C::SocialAccess, 63)],
        &[],
    )?;
    for member in [lieutenant, enforcer, burglar, scout] {
        validate_set_relationship(&state, member, boss, relationship(70, 78, 58))?
            .commit(&mut state)?;
    }
    validate_set_relationship(&state, boss, detectives[0], relationship(65, 62, 28))?
        .commit(&mut state)?;
    validate_set_relationship(&state, boss, lawyer, relationship(74, 79, 35))?
        .commit(&mut state)?;
    validate_set_relationship(&state, lieutenant, lawyer, relationship(65, 73, 23))?
        .commit(&mut state)?;
    for contact in [councilor, union_delegate, merchant] {
        validate_set_relationship(&state, boss, contact, relationship(61, 68, 32))?
            .commit(&mut state)?;
    }

    // Rival outfits have real personnel, treasury, and independent leadership.
    // Their neighborhood mandates let the autonomous recruitment/enterprise
    // passes compete with the player after time starts moving.
    let mut rival_leaders = Vec::new();
    for (index, rival) in rivals.iter().copied().enumerate() {
        let leader = add_character(
            &mut state,
            person_name(&mut rng, 250 + index),
            Some(rival),
            None,
            AutonomyLevel::Broad,
            &[(C::Management, 77), (C::Negotiation, 72)],
            &[TraitKind::Ambitious],
        )?;
        add_character(
            &mut state,
            person_name(&mut rng, 300 + index),
            Some(rival),
            Some(leader),
            AutonomyLevel::Guided,
            &[(C::Violence, 75), (C::Intimidation, 73)],
            &[],
        )?;
        add_character(
            &mut state,
            person_name(&mut rng, 350 + index),
            Some(rival),
            Some(leader),
            AutonomyLevel::Guided,
            &[(C::Driving, 73), (C::Surveillance, 72)],
            &[],
        )?;
        validate_assign_mandate(
            &state,
            MandateDraft {
                organization: rival,
                manager: leader,
                scopes: BTreeSet::from([
                    ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                    ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
                    ResponsibilityScope::Neighborhood(districts[index + 1]),
                ]),
                standing_orders: BTreeMap::from([(
                    PolicyKind::IndependentRecruitment,
                    PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
                )]),
                budget: None,
            },
        )?
        .commit(&mut state)?;
        rival_leaders.push(leader);
    }
    let player_mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: player,
            manager: lieutenant,
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(home),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )?
    .commit(&mut state)?;
    validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor: player,
            handler: boss,
            contact: detectives[0],
        },
    )?
    .commit(&mut state)?;

    // Civilian owner-occupiers and recruits exist independently of the criminal
    // crews; their underlying motives are not injected into the player's view.
    let mut civilians = Vec::new();
    for index in 0..usize::from(config.districts) {
        let owner = add_character(
            &mut state,
            person_name(&mut rng, 400 + index),
            None,
            None,
            AutonomyLevel::Guided,
            &[(C::SocialAccess, 55)],
            &[],
        )?;
        civilians.push(owner);
        let prospect = add_character(
            &mut state,
            person_name(&mut rng, 500 + index),
            None,
            None,
            AutonomyLevel::Guided,
            &[(C::Burglary, 65), (C::Stealth, 63), (C::Driving, 59)],
            &[TraitKind::Greedy],
        )?;
        if index < 2 {
            validate_set_relationship(&state, prospect, boss, relationship(78, 76, 62))?
                .commit(&mut state)?;
        } else {
            let rival_index = (index - 2) % rivals.len();
            validate_set_relationship(
                &state,
                prospect,
                rival_leaders[rival_index],
                relationship(71, 68, 42),
            )?
            .commit(&mut state)?;
        }
    }

    // Place authored *roles*, not authored properties: every city has playable
    // opening infrastructure, but its names, district conditions and surrounding
    // industries are generated. Remaining businesses mix all 25 registry kinds.
    let mut primary_target = None;
    let mut secondary_target = None;
    let mut starter_front = None;
    let mut rival_fronts = Vec::new();
    for (district_index, district) in districts.iter().copied().enumerate() {
        let district_name = state
            .world()
            .get_neighborhood(district)
            .expect("newly inserted district must exist")
            .name()
            .to_owned();
        for slot in 0..usize::from(config.businesses_per_district) {
            let (name, kind, owner) = if district_index == 0 {
                match slot {
                    0 => (
                        format!("{} Jewelers", SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())]),
                        K::Retail,
                        BusinessOwner::Character(civilians[0]),
                    ),
                    1 => (
                        format!(
                            "{} Receiving House",
                            SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())]
                        ),
                        K::Retail,
                        BusinessOwner::Independent,
                    ),
                    2 => (
                        format!(
                            "{} Social Club",
                            SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())]
                        ),
                        K::Hospitality,
                        BusinessOwner::Organization(player),
                    ),
                    3 => (
                        format!(
                            "{} Pawn Exchange",
                            SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())]
                        ),
                        K::Pawnshop,
                        BusinessOwner::Organization(player),
                    ),
                    _ => random_business(
                        &mut rng,
                        district_index,
                        slot,
                        &district_name,
                        &rivals,
                        &civilians,
                    ),
                }
            } else if district_index <= rivals.len() && slot == 0 {
                (
                    format!(
                        "{} Social Club",
                        SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())]
                    ),
                    K::Hospitality,
                    BusinessOwner::Organization(rivals[district_index - 1]),
                )
            } else {
                random_business(
                    &mut rng,
                    district_index,
                    slot,
                    &district_name,
                    &rivals,
                    &civilians,
                )
            };
            let (_, functions) = business_template(kind);
            let id = add_business(
                registry,
                &mut state,
                format!("{name} ({district_name}, {})", slot + 1),
                kind,
                functions,
                district,
                owner,
            )?;
            if district_index == 0 {
                match slot {
                    0 => primary_target = Some(id),
                    1 => secondary_target = Some(id),
                    2 => starter_front = Some(id),
                    _ => {}
                }
            } else if district_index <= rivals.len() && slot == 0 {
                rival_fronts.push(id);
            }
        }
    }
    let primary_target =
        primary_target.expect("validated business density includes a primary target");
    let secondary_target =
        secondary_target.expect("validated business density includes a second target");
    let starter_front = starter_front.expect("validated density includes an initial cash front");

    let (cash, settlement) = treasury(&mut state, player, 150_000)?;
    let owner = FinancialOwner::Organization(player);
    add_account(&mut state, owner, AccountKind::AccountedFunds)?;
    add_account(&mut state, owner, AccountKind::ConcealedCash)?;
    add_account(&mut state, owner, AccountKind::StreetCash)?;
    add_account(&mut state, owner, AccountKind::Settlement)?;
    let starter_racket = validate_establish_enterprise(
        registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: player,
            authority: MandateAuthority {
                mandate: player_mandate,
                manager: lieutenant,
                scope: ResponsibilityScope::Neighborhood(home),
            },
            location: EnterpriseLocation::Business(starter_front),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )?
    .commit(&mut state)?;
    for (index, rival) in rivals.iter().copied().enumerate() {
        let (cash, settlement) = treasury(&mut state, rival, 100_000)?;
        let manager = rival_leaders[index];
        let mandate = state
            .delegation()
            .active_for_manager(manager)
            .expect("generated rival manager has a validated mandate")
            .id();
        validate_establish_enterprise(
            registry,
            &state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization: rival,
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope: ResponsibilityScope::Neighborhood(districts[index + 1]),
                },
                location: EnterpriseLocation::Business(rival_fronts[index]),
                supporting_businesses: BTreeSet::new(),
                cash_account: cash,
                settlement_account: settlement,
            },
        )?
        .commit(&mut state)?;
    }

    for (person, name) in [
        (lawyer, "defense lawyer"),
        (councilor, "council member"),
        (union_delegate, "labor representative"),
        (merchant, "commercial representative"),
    ] {
        let organization = state
            .world()
            .get_character(person)
            .and_then(|character| character.organization())
            .expect("generated institutional representative has an organization");
        let institution = state
            .world()
            .get_organization(organization)
            .expect("representative's institution exists")
            .name();
        let summary = format!(
            "A known {name} works with {institution} and has a personal relationship to leadership."
        );
        validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(player),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: Some(EntityRef::Character(boss)),
                subject: EntityRef::Character(person),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary,
            },
        )?
        .commit(&mut state)?;
    }
    for (index, target) in [primary_target, secondary_target].into_iter().enumerate() {
        let name = state
            .world()
            .get_business(target)
            .expect("generated target exists")
            .name()
            .to_owned();
        let fact = validate_record_information(&state, InformationDraft {
            holder: KnowledgeHolder::Organization(player),
            source_kind: if index == 0 {
                InformationSourceKind::StreetRumor
            } else {
                InformationSourceKind::DirectObservation
            },
            topic: InformationTopic::TargetSecurity,
            source_entity: Some(EntityRef::Character(if index == 0 { civilians[0] } else { scout })),
            subject: EntityRef::Business(target),
            observed_at: state.now(),
            reliability: if index == 0 { Reliability::GenerallyReliable } else { Reliability::DirectAccess },
            specificity: if index == 0 { Specificity::Specific } else { Specificity::Precise },
            summary: format!("A local source identified a potential after-hours opening at {name}; the current alarm and patrol response are not known."),
        })?.commit(&mut state)?;
        validate_discover_operation_opportunity(
            registry,
            &state,
            OperationOpportunityDraft {
                organization: player,
                operation_kind: OperationKind::Burglary,
                targets: BTreeSet::from([EntityRef::Business(target)]),
                source_information: BTreeSet::from([fact]),
                summary: format!("Investigate after-hours access to {name}."),
                valid_until: (index == 0).then_some(SimTime::from_minutes(720)),
            },
        )?
        .commit(&mut state)?;
    }

    crate::core::invariants::validate_state(&state)?;
    crate::core::invariants::validate_state_against_registry(registry, &state)?;
    Ok(GeneratedCity {
        state,
        player,
        home,
        boss,
        primary_target,
        secondary_target,
        starter_front,
        starter_racket,
        police,
        rivals,
        seed: config.seed,
    })
}

fn random_business(
    rng: &mut CityRng,
    district_index: usize,
    slot: usize,
    ward_name: &str,
    rivals: &[OrganizationId],
    civilians: &[CharacterId],
) -> (String, K, BusinessOwner) {
    // Six industrial identities generate recognizable local clusters without
    // excluding any of the 25 modeled industry kinds elsewhere in the city.
    let thematic = match district_index % 6 {
        0 => [
            K::Retail,
            K::FinancialServices,
            K::Pharmacy,
            K::MovieTheater,
        ],
        1 => [
            K::Construction,
            K::Automotive,
            K::GarmentFactory,
            K::Printing,
        ],
        2 => [K::Stevedoring, K::Warehouse, K::Transportation, K::Brewery],
        3 => [K::Lodging, K::SoupKitchen, K::Laundry, K::Retail],
        4 => [
            K::Nightclub,
            K::Hospitality,
            K::MovieTheater,
            K::AthleticClub,
        ],
        _ => [K::TaxiGarage, K::Pawnshop, K::Wholesale, K::NewsService],
    };
    let kind = if rng.range(100) < 62 {
        thematic[(slot + rng.range(4)) % 4]
    } else {
        ALL_BUSINESS_KINDS[rng.range(ALL_BUSINESS_KINDS.len())]
    };
    let owner = match rng.range(100) {
        0..=17 => BusinessOwner::Organization(rivals[(district_index + slot) % rivals.len()]),
        18..=30 => BusinessOwner::Character(civilians[district_index]),
        _ => BusinessOwner::Independent,
    };
    let (suffix, _) = business_template(kind);
    let name = format!(
        "{} {} of {}",
        SHOP_PREFIXES[rng.range(SHOP_PREFIXES.len())],
        suffix,
        ward_name
    );
    (name, kind, owner)
}

#[cfg(test)]
mod tests;
