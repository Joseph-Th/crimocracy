//! Seeded commercial geography, private proprietors and playable opening properties.
//! Every property and its operating accounts are created by the canonical owners.

use super::{
    CityGenerationConfig, CityRng, GenerationResult, add_account, add_character, person_name,
    relationship,
};
use crate::core::id::{BusinessId, CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::finance::{AccountKind, FinancialOwner};
use crate::registry::Registry;
use crate::social::relationship_system::validate_set_relationship;
use crate::world::world_system::insert_business;
use crate::world::{
    ALL_BUSINESS_KINDS, AutonomyLevel, BusinessDraft, BusinessFunction as F, BusinessKind as K,
    BusinessOwner, CapabilityKind as C, TraitKind,
};

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

pub(super) struct PropertyCity<'a> {
    pub(super) config: CityGenerationConfig,
    pub(super) districts: &'a [NeighborhoodId],
    pub(super) player: OrganizationId,
    pub(super) rivals: &'a [OrganizationId],
    pub(super) civilians: &'a [CharacterId],
    pub(super) merchant: CharacterId,
}

pub(super) struct GeneratedProperties {
    pub(super) primary_target: BusinessId,
    pub(super) secondary_target: BusinessId,
    pub(super) starter_front: BusinessId,
    pub(super) rival_fronts: Vec<BusinessId>,
}

pub(super) fn generate_properties(
    registry: &Registry,
    state: &mut AppState,
    rng: &mut CityRng,
    context: PropertyCity<'_>,
) -> GenerationResult<GeneratedProperties> {
    let PropertyCity {
        config,
        districts,
        player,
        rivals,
        civilians,
        merchant,
    } = context;
    // Place authored *roles*, not authored properties: every city has playable
    // opening infrastructure, but its names, district conditions and surrounding
    // industries are generated. Remaining businesses mix all 25 registry kinds.
    let mut primary_target = None;
    let mut secondary_target = None;
    let mut starter_front = None;
    let mut rival_fronts = Vec::new();
    // Personal biographies must not alter the seeded property mix or the
    // documented public listing opportunities by consuming geography RNG.
    let mut proprietor_rng = CityRng::new(config.seed ^ 0xB89D_0A31_2026_F17E);
    proprietor_rng.used_names = rng.used_names.clone();
    for (district_index, district) in districts.iter().copied().enumerate() {
        let district_name = state
            .world()
            .get_neighborhood(district)
            .expect("newly inserted district must exist")
            .name()
            .to_owned();
        for slot in 0..usize::from(config.businesses_per_district) {
            let (name, kind, mut owner) = if district_index == 0 {
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
                        rng,
                        district_index,
                        slot,
                        &district_name,
                        rivals,
                        civilians,
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
                random_business(rng, district_index, slot, &district_name, rivals, civilians)
            };
            // A private operating shop belongs to a particular person. Only
            // separately advertised public listings retain Independent title
            // until purchase; they are not anonymous simulated proprietors.
            // Introduce these people at their actual shop's creation, through
            // the canonical world owner and with their own accountable finances.
            if owner == BusinessOwner::Independent
                && !(district_index == 0 && slot == 1)
                && kind != K::SoupKitchen
                && (slot + district_index * 3) % 2 == 0
            {
                let proprietor = add_character(
                    state,
                    person_name(&mut proprietor_rng),
                    None,
                    None,
                    AutonomyLevel::Guided,
                    &[
                        (C::Management, 37 + proprietor_rng.range(54) as u8),
                        (C::Negotiation, 36 + proprietor_rng.range(53) as u8),
                        (C::SocialAccess, 32 + proprietor_rng.range(58) as u8),
                    ],
                    &[if proprietor_rng.range(2) == 0 {
                        TraitKind::Cautious
                    } else {
                        TraitKind::Ambitious
                    }],
                )?;
                validate_set_relationship(
                    state,
                    proprietor,
                    merchant,
                    relationship(47 + proprietor_rng.range(25) as u8, 55, 27),
                )?
                .commit(state)?;
                owner = BusinessOwner::Character(proprietor);
            }
            let (_, functions) = business_template(kind);
            let id = add_business(
                registry,
                state,
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
    // Reserve the names of all newly created owners before drawing the
    // additional civic employees, preventing duplicate identities without
    // altering the city's preceding economic geography.
    rng.used_names.extend(proprietor_rng.used_names);
    let primary_target =
        primary_target.expect("validated business density includes a primary target");
    let secondary_target =
        secondary_target.expect("validated business density includes a second target");
    let starter_front = starter_front.expect("validated density includes an initial cash front");

    Ok(GeneratedProperties {
        primary_target,
        secondary_target,
        starter_front,
        rival_fronts,
    })
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
        K::Automotive => (
            "Auto Works",
            &[VehicleWorkshop, CustomerAccess, ResaleMarket],
        ),
        K::Transportation => ("Freight", &[VehicleFleet, DistributionInfrastructure]),
        K::Warehouse => ("Storage", &[Warehousing, DistributionInfrastructure]),
        K::ProfessionalServices => ("Accounting", &[ProfessionalRecords, CustomerAccess]),
        K::Brewery => ("Brewery", &[AlcoholProduction, Warehousing]),
        K::Nightclub => (
            "Nightclub",
            &[CashIntensive, Nightlife, MeetingSpace, CustomerAccess],
        ),
        K::Construction => ("Builders", &[VehicleFleet, Warehousing, UnionAccess]),
        K::Wholesale => (
            "Wholesale",
            &[Warehousing, DistributionInfrastructure, CustomerAccess],
        ),
        K::Pawnshop => (
            "Exchange",
            &[CashIntensive, ResaleMarket, CustomerAccess, Warehousing],
        ),
        K::CoinMachineDistribution => (
            "Coin Machines",
            &[
                DistributionInfrastructure,
                CashIntensive,
                VehicleFleet,
                CustomerAccess,
            ],
        ),
        K::Lodging => (
            "Hotel",
            &[Lodging, CustomerAccess, MeetingSpace, CashIntensive],
        ),
        K::AthleticClub => ("Gymnasium", &[SportingVenue, MeetingSpace, CustomerAccess]),
        K::Laundry => ("Laundry", &[CashIntensive, CustomerAccess]),
        K::Printing => ("Printing", &[PrintingPress, ProfessionalRecords]),
        K::FinancialServices => ("Savings", &[FinancialServices, ProfessionalRecords]),
        K::GarmentFactory => (
            "Textiles",
            &[Warehousing, DistributionInfrastructure, UnionAccess],
        ),
        K::Stevedoring => (
            "Dock Services",
            &[
                DockAccess,
                Warehousing,
                UnionAccess,
                DistributionInfrastructure,
            ],
        ),
        K::NewsService => ("Racing Wire", &[RacingWire, ProfessionalRecords]),
        K::Pharmacy => ("Pharmacy", &[CustomerAccess, ProfessionalRecords]),
        K::FuneralParlor => ("Funeral Home", &[MeetingSpace, CustomerAccess]),
        K::SoupKitchen => ("Kitchen", &[CustomerAccess, CashIntensive]),
        K::TaxiGarage => ("Taxi Garage", &[VehicleFleet, VehicleWorkshop]),
        K::MovieTheater => ("Picture House", &[CustomerAccess, CashIntensive]),
    }
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
