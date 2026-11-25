//! District population construction, including workplaces and institutional appointments.
//! Uses a separate seeded stream and canonical character, occupation and relationship owners.

use super::{CityRng, GenerationResult, person_name, rating, relationship};
use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::social::relationship_system::validate_set_relationship;
use crate::world::world_system::insert_character;
use crate::world::{
    AutonomyLevel, BusinessOwner, CapabilityKind as C, CharacterDraft, DriveKind,
    IndependentVocation, TraitKind,
};
use std::collections::{BTreeMap, BTreeSet};

struct PopulationRole<'a> {
    vocation: IndependentVocation,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    skills: &'a [(C, u8)],
    personality: TraitKind,
    contact: Option<CharacterId>,
}

impl<'a> PopulationRole<'a> {
    fn new(
        vocation: IndependentVocation,
        organization: Option<OrganizationId>,
        supervisor: Option<CharacterId>,
        skills: &'a [(C, u8)],
        personality: TraitKind,
        contact: Option<CharacterId>,
    ) -> Self {
        Self {
            vocation,
            organization,
            supervisor,
            skills,
            personality,
            contact,
        }
    }
}

struct PopulationPerson<'a> {
    home: NeighborhoodId,
    vocation: IndependentVocation,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    capabilities: &'a [(C, u8)],
    trait_kind: TraitKind,
}

fn add_population_person(
    state: &mut AppState,
    rng: &mut CityRng,
    profile: PopulationPerson<'_>,
) -> GenerationResult<CharacterId> {
    let PopulationPerson {
        home,
        vocation,
        organization,
        supervisor,
        capabilities,
        trait_kind,
    } = profile;
    let name = person_name(rng);
    let person = insert_character(
        state,
        CharacterDraft {
            name,
            organization,
            supervisor,
            autonomy: if organization.is_some() {
                AutonomyLevel::Guided
            } else {
                AutonomyLevel::Broad
            },
            capabilities: capabilities
                .iter()
                .map(|&(kind, value)| (kind, rating(value)))
                .collect(),
            traits: BTreeSet::from([trait_kind]),
            drives: BTreeMap::from([
                (DriveKind::Money, rng.rating(25, 96)),
                (DriveKind::Safety, rng.rating(22, 95)),
                (DriveKind::Status, rng.rating(20, 94)),
                (DriveKind::Independence, rng.rating(20, 98)),
            ]),
        },
    )?;
    crate::world::character_life::validate_set_civic_occupation(state, person, home, vocation)?
        .commit(state)?;
    Ok(person)
}

pub(super) struct PopulationCity<'a> {
    pub(super) districts: &'a [NeighborhoodId],
    pub(super) police: &'a [OrganizationId],
    pub(super) detectives: &'a [CharacterId],
    pub(super) council: OrganizationId,
    pub(super) councilor: CharacterId,
    pub(super) legal_services: OrganizationId,
    pub(super) lawyer: CharacterId,
    pub(super) prosecutor: OrganizationId,
    pub(super) labor: OrganizationId,
    pub(super) union_delegate: CharacterId,
    pub(super) commerce: OrganizationId,
    pub(super) merchant: CharacterId,
    pub(super) boss: CharacterId,
    pub(super) lieutenant: CharacterId,
    pub(super) scout: CharacterId,
    pub(super) rivals: &'a [OrganizationId],
    pub(super) rival_leaders: &'a [CharacterId],
    pub(super) reserved_names: &'a BTreeSet<String>,
}

pub(super) fn populate_city(
    state: &mut AppState,
    seed: u64,
    context: PopulationCity<'_>,
) -> GenerationResult<()> {
    let PopulationCity {
        districts,
        police,
        detectives,
        council,
        councilor,
        legal_services,
        lawyer,
        prosecutor,
        labor,
        union_delegate,
        commerce,
        merchant,
        boss,
        lieutenant,
        scout,
        rivals,
        rival_leaders,
        reserved_names,
    } = context;
    let mut rng = CityRng::new(seed ^ 0x9CE5_27AD_1347_8F10);
    rng.used_names = reserved_names.clone();
    for (&rival, &leader) in rivals.iter().zip(rival_leaders) {
        for index in 0..2 {
            let member = add_population_person(
                state,
                &mut rng,
                PopulationPerson {
                    home: districts[(index + 1) % districts.len()],
                    vocation: IndependentVocation::Resident,
                    organization: Some(rival),
                    supervisor: Some(leader),
                    capabilities: if index == 0 {
                        &[(C::Violence, 65), (C::Intimidation, 66)]
                    } else {
                        &[(C::Burglary, 67), (C::Stealth, 72), (C::Driving, 61)]
                    },
                    trait_kind: TraitKind::LoyalToFamily,
                },
            )?;
            validate_set_relationship(state, member, leader, relationship(69, 74, 57))?
                .commit(state)?;
        }
    }
    for (ward_index, &ward) in districts.iter().enumerate() {
        let precinct = ward_index % police.len();
        let shops: Vec<_> = state
            .world()
            .businesses_in_neighborhood(ward)
            .map(|business| (business.id(), business.owner()))
            .collect();
        // Preserve the usual staggered workplace order, but use a full traversal
        // when a custom density shares a factor with the normal stride of five.
        // Each workplace must be visited before another employee repeats it.
        let workplace_stride = if shops.len().is_multiple_of(5) { 1 } else { 5 };
        for slot in 0..42 {
            let workplace = shops[(slot * workplace_stride + ward_index) % shops.len()];
            let PopulationRole {
                vocation,
                organization,
                supervisor,
                skills,
                personality,
                contact,
            } = match slot {
                0..=11 => PopulationRole::new(
                    IndependentVocation::BusinessEmployee(workplace.0),
                    None,
                    None,
                    &[(C::Management, 47), (C::SocialAccess, 56)],
                    if slot % 3 == 0 {
                        TraitKind::Ambitious
                    } else {
                        TraitKind::Cautious
                    },
                    Some(match workplace.1 {
                        BusinessOwner::Character(person) => person,
                        BusinessOwner::Independent | BusinessOwner::Organization(_) => merchant,
                    }),
                ),
                12..=20 => {
                    let n = slot - 12;
                    let skills: &[(C, u8)] = match n % 5 {
                        0 => &[(C::Burglary, 74), (C::Stealth, 81)],
                        1 => &[(C::Violence, 75), (C::Intimidation, 68)],
                        2 => &[(C::Driving, 84), (C::Surveillance, 63)],
                        3 => &[(C::Surveillance, 77), (C::Stealth, 74)],
                        _ => &[(C::Negotiation, 69), (C::Intimidation, 70)],
                    };
                    let contact = match n {
                        0 => boss,
                        1 => lieutenant,
                        2 => scout,
                        3 | 4 => rival_leaders[(ward_index + n) % rival_leaders.len()],
                        _ => merchant,
                    };
                    PopulationRole::new(
                        IndependentVocation::FreelanceCriminal,
                        None,
                        None,
                        skills,
                        if n % 2 == 0 {
                            TraitKind::Greedy
                        } else {
                            TraitKind::Secretive
                        },
                        Some(contact),
                    )
                }
                21..=28 => PopulationRole::new(
                    IndependentVocation::Resident,
                    None,
                    None,
                    &[(C::SocialAccess, 49)],
                    TraitKind::LoyalToFamily,
                    Some(if slot % 2 == 0 {
                        union_delegate
                    } else {
                        merchant
                    }),
                ),
                29..=32 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(police[precinct]),
                    Some(detectives[precinct]),
                    &[
                        (C::Investigation, 68),
                        (C::Surveillance, 67),
                        (C::Violence, 48),
                    ],
                    TraitKind::Cautious,
                    Some(detectives[precinct]),
                ),
                33..=34 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(council),
                    Some(councilor),
                    &[(C::Negotiation, 74), (C::SocialAccess, 76)],
                    TraitKind::Ambitious,
                    Some(councilor),
                ),
                35..=36 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(legal_services),
                    Some(lawyer),
                    &[(C::LegalKnowledge, 78), (C::Negotiation, 66)],
                    TraitKind::Proud,
                    Some(lawyer),
                ),
                37 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(prosecutor),
                    None,
                    &[(C::LegalKnowledge, 79), (C::Investigation, 68)],
                    TraitKind::Cautious,
                    None,
                ),
                38 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(labor),
                    Some(union_delegate),
                    &[(C::Negotiation, 74), (C::SocialAccess, 72)],
                    TraitKind::LoyalToFamily,
                    Some(union_delegate),
                ),
                39 => PopulationRole::new(
                    IndependentVocation::Resident,
                    Some(commerce),
                    Some(merchant),
                    &[(C::Management, 66), (C::Negotiation, 72)],
                    TraitKind::Charismatic,
                    Some(merchant),
                ),
                40 => PopulationRole::new(
                    IndependentVocation::Journalist,
                    None,
                    None,
                    &[(C::Surveillance, 79), (C::SocialAccess, 68)],
                    TraitKind::Secretive,
                    Some(merchant),
                ),
                41 => PopulationRole::new(
                    IndependentVocation::IndependentLawyer,
                    None,
                    None,
                    &[(C::LegalKnowledge, 82), (C::Negotiation, 72)],
                    TraitKind::Proud,
                    Some(lawyer),
                ),
                _ => unreachable!("finite local population role table"),
            };
            let person = add_population_person(
                state,
                &mut rng,
                PopulationPerson {
                    home: ward,
                    vocation,
                    organization,
                    supervisor,
                    capabilities: skills,
                    trait_kind: personality,
                },
            )?;
            if let Some(contact) = contact {
                validate_set_relationship(state, person, contact, relationship(55, 64, 32))?
                    .commit(state)?;
            }
        }
    }
    Ok(())
}
