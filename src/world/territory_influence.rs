//! Read-only per-district influence aggregation over canonical world, enterprise, and
//! delegation records; no mutation paths live here.
//!
//! Territory answers "who holds sway in this district" from three record-backed dimensions
//! ([`GAME_DESIGN.md`] §25): economic presence (active enterprises operated inside the
//! district), property holdings (district businesses owned outright), and governance (an
//! active mandate scoped to the district itself). Every figure is derived from authoritative
//! records at resolution time; nothing about influence is persisted.
//!
//! Consumers are simulation-side decision makers — delegated expansion, future contest
//! behavior, campaign end-state evaluation. This summary is not a player information feed:
//! surfacing it verbatim would bypass the provenance-bearing intelligence model.

use crate::core::id::{NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::MandateStatus;
use crate::enterprises::{EnterpriseKind, EnterpriseLocation, EnterpriseStatus};
use crate::world::{BusinessOwner, Lifecycle};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TerritoryInfluenceError {
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
}

/// One organization's record-backed footprint inside a single district.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerritoryStanding {
    pub organization: OrganizationId,
    /// Active enterprises operating at district locations, regardless of host ownership.
    pub active_enterprises: u32,
    /// Distinct racket kinds among those enterprises.
    pub enterprise_kinds: BTreeSet<EnterpriseKind>,
    /// Active district businesses owned outright by the organization.
    pub owned_venues: u32,
    /// Whether an active mandate scoped to this district governs the organization's play here.
    pub governed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborhoodInfluenceSummary {
    pub neighborhood: NeighborhoodId,
    /// Ascending organization order; organizations with no district footprint are absent.
    pub standings: Vec<TerritoryStanding>,
}

impl NeighborhoodInfluenceSummary {
    pub fn standing_for(&self, organization: OrganizationId) -> Option<&TerritoryStanding> {
        self.standings
            .iter()
            .find(|standing| standing.organization == organization)
    }

    /// The unique strict leader on active-enterprise count, if any. Contested districts —
    /// ties between two or more organizations — have no leader, because territory is
    /// contested rather than shared by default.
    pub fn economic_leader(&self) -> Option<OrganizationId> {
        let mut leader: Option<(u32, OrganizationId)> = None;
        let mut tied = false;
        for standing in &self.standings {
            let count = standing.active_enterprises;
            if count == 0 {
                continue;
            }
            match leader {
                None => {
                    leader = Some((count, standing.organization));
                    tied = false;
                }
                Some((best_count, _)) if best_count < count => {
                    leader = Some((count, standing.organization));
                    tied = false;
                }
                Some((best_count, _)) if best_count == count => tied = true,
                Some(_) => {}
            }
        }
        if tied {
            None
        } else {
            leader.map(|(_, organization)| organization)
        }
    }
}

/// Resolves who holds sway in a district from canonical records alone. Deterministic and
/// allocation-stable: standings ascend by organization id, enterprise kinds sort inherently.
pub fn resolve_neighborhood_influence(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Result<NeighborhoodInfluenceSummary, TerritoryInfluenceError> {
    if state.world().get_neighborhood(neighborhood).is_none() {
        return Err(TerritoryInfluenceError::MissingNeighborhood(neighborhood));
    }

    // Economic presence: active enterprises whose hosted or district location resolves to
    // this neighborhood. Ordered scans keep the derivation deterministic.
    let mut enterprises_by_org: BTreeMap<OrganizationId, u32> = BTreeMap::new();
    let mut kinds_by_org: BTreeMap<OrganizationId, BTreeSet<EnterpriseKind>> = BTreeMap::new();
    for record in state.enterprises().enterprises() {
        if record.status() != EnterpriseStatus::Active {
            continue;
        }
        let in_district = match record.location() {
            EnterpriseLocation::Neighborhood(id) => id == neighborhood,
            EnterpriseLocation::Business(business) => state
                .world()
                .get_business(business)
                .is_some_and(|host| host.neighborhood() == neighborhood),
        };
        if !in_district {
            continue;
        }
        let organization = record.organization();
        *enterprises_by_org.entry(organization).or_default() += 1;
        kinds_by_org
            .entry(organization)
            .or_default()
            .insert(record.kind());
    }

    // Property holdings: active district businesses owned outright.
    let mut venues_by_org: BTreeMap<OrganizationId, u32> = BTreeMap::new();
    for business in state.world().businesses_in_neighborhood(neighborhood) {
        if business.lifecycle() != Lifecycle::Active {
            continue;
        }
        if let BusinessOwner::Organization(organization) = business.owner() {
            *venues_by_org.entry(organization).or_default() += 1;
        }
    }

    // Governance: an active territorial mandate scoped to the district itself.
    let governed: BTreeSet<OrganizationId> = state
        .delegation()
        .mandates()
        .filter(|mandate| mandate.status() == MandateStatus::Active)
        .filter(|mandate| {
            mandate.scopes().iter().any(|scope| {
                *scope == crate::delegation::ResponsibilityScope::Neighborhood(neighborhood)
            })
        })
        .map(|mandate| mandate.organization())
        .collect();

    let mut organizations: Vec<OrganizationId> = enterprises_by_org
        .keys()
        .copied()
        .chain(venues_by_org.keys().copied())
        .chain(governed.iter().copied())
        .collect();
    organizations.sort_unstable();
    organizations.dedup();

    let standings = organizations
        .into_iter()
        .map(|organization| TerritoryStanding {
            organization,
            active_enterprises: enterprises_by_org
                .get(&organization)
                .copied()
                .unwrap_or_default(),
            enterprise_kinds: kinds_by_org.get(&organization).cloned().unwrap_or_default(),
            owned_venues: venues_by_org
                .get(&organization)
                .copied()
                .unwrap_or_default(),
            governed: governed.contains(&organization),
        })
        .collect();

    Ok(NeighborhoodInfluenceSummary {
        neighborhood,
        standings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::id::{FinancialAccountId, MandateId};
    use crate::core::invariants::validate_invariants;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::ResponsibilityScope;
    use crate::delegation::{MandateAuthority, MandateDraft};
    use crate::enterprises::enterprise_execution::{
        validate_establish_enterprise, validate_suspend_enterprise,
    };
    use crate::enterprises::EnterpriseDraft;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::registry::Registry;
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, CharacterDraft,
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::BTreeSet;

    struct InfluenceFixture {
        state: AppState,
        neighborhood: NeighborhoodId,
        dominant: OrganizationId,
        challenger: OrganizationId,
        /// The dominant organization's district-scoped mandate.
        mandate: MandateId,
        manager: crate::core::id::CharacterId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn make_influence_fixture() -> InfluenceFixture {
        let registry = build_registry();
        let mut state = AppState::new(0x7E27_1A11);
        let dominant = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Dominant Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("dominant organization should validate");
        let challenger = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Challenger Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("challenger organization should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Influence Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(55),
                        commercial_activity: rating(60),
                        illicit_demand: rating(45),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(40),
                        political_influence: rating(50),
                        social_cohesion: rating(60),
                        visible_violence_tolerance: rating(25),
                    },
                },
            },
        )
        .expect("neighborhood should validate");
        let manager = insert_character(
            &mut state,
            CharacterDraft {
                name: "District Lieutenant".to_owned(),
                organization: Some(dominant),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("manager should validate");
        let mandate = validate_assign_mandate(
            &state,
            MandateDraft {
                organization: dominant,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("mandate should validate")
        .commit(&mut state)
        .expect("mandate should commit");
        InfluenceFixture {
            state,
            neighborhood,
            dominant,
            challenger,
            mandate,
            manager,
        }
    }

    fn org_cash(state: &mut AppState, organization: OrganizationId) -> FinancialAccountId {
        insert_account(
            state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
            },
        )
        .expect("cash account should validate")
    }

    fn fresh_settlement(state: &mut AppState, organization: OrganizationId) -> FinancialAccountId {
        insert_account(
            state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
            },
        )
        .expect("settlement account should validate")
    }

    fn insert_venue(
        registry: &Registry,
        state: &mut AppState,
        name: &str,
        neighborhood: NeighborhoodId,
        owner: BusinessOwner,
        functions: BTreeSet<BusinessFunction>,
    ) -> crate::core::id::BusinessId {
        insert_business(
            registry,
            state,
            BusinessDraft {
                name: name.to_owned(),
                kind: BusinessKind::Hospitality,
                functions,
                neighborhood,
                owner,
            },
        )
        .expect("venue should validate")
    }

    fn establish_gambling_at_venue(
        registry: &Registry,
        state: &mut AppState,
        organization: OrganizationId,
        mandate: MandateId,
        manager: crate::core::id::CharacterId,
        venue: crate::core::id::BusinessId,
    ) -> crate::core::id::EnterpriseId {
        let scope = ResponsibilityScope::Neighborhood(
            state
                .world()
                .get_business(venue)
                .expect("hosted venue should exist")
                .neighborhood(),
        );
        let cash_account = org_cash(state, organization);
        let settlement_account = fresh_settlement(state, organization);
        validate_establish_enterprise(
            registry,
            state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization,
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope,
                },
                location: EnterpriseLocation::Business(venue),
                supporting_businesses: BTreeSet::new(),
                cash_account,
                settlement_account,
            },
        )
        .expect("gambling establishment should validate")
        .commit(state)
        .expect("gambling establishment should commit")
    }

    fn hospitality_functions() -> BTreeSet<BusinessFunction> {
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ])
    }

    #[test]
    fn influence_derives_economic_property_and_governance_dimensions_from_records() {
        let registry = build_registry();
        let mut fixture = make_influence_fixture();

        let dominant_venue = insert_venue(
            &registry,
            &mut fixture.state,
            "Dominant Hall",
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.dominant),
            hospitality_functions(),
        );
        let _challenger_venue = insert_venue(
            &registry,
            &mut fixture.state,
            "Challenger Corner",
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.challenger),
            hospitality_functions(),
        );
        establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            dominant_venue,
        );

        let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
            .expect("influence should resolve");
        assert_eq!(summary.neighborhood, fixture.neighborhood);
        assert_eq!(
            summary.standings.len(),
            2,
            "both organizations hold district footprint"
        );
        assert!(
            summary.standings[0].organization < summary.standings[1].organization,
            "standings ascend by organization id"
        );

        let dominant = summary
            .standing_for(fixture.dominant)
            .expect("dominant standing should exist");
        assert_eq!(dominant.active_enterprises, 1);
        assert!(dominant
            .enterprise_kinds
            .contains(&EnterpriseKind::Gambling));
        assert_eq!(dominant.owned_venues, 1);
        assert!(dominant.governed);

        let challenger = summary
            .standing_for(fixture.challenger)
            .expect("challenger standing should exist");
        assert_eq!(challenger.active_enterprises, 0);
        assert!(challenger.enterprise_kinds.is_empty());
        assert_eq!(challenger.owned_venues, 1);
        assert!(!challenger.governed);

        assert_eq!(
            summary.economic_leader(),
            Some(fixture.dominant),
            "the sole racket operator leads an uncontested district"
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn contested_districts_have_no_economic_leader() {
        let registry = build_registry();
        let mut fixture = make_influence_fixture();

        // A second district-scoped mandate lets the challenger operate here too.
        let challenger_manager = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "Challenger Lieutenant".to_owned(),
                organization: Some(fixture.challenger),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("challenger manager should validate");
        let challenger_mandate = validate_assign_mandate(
            &fixture.state,
            MandateDraft {
                organization: fixture.challenger,
                manager: challenger_manager,
                scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(fixture.neighborhood)]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("challenger mandate should validate")
        .commit(&mut fixture.state)
        .expect("challenger mandate should commit");

        for (organization, mandate, manager, name) in [
            (
                fixture.dominant,
                fixture.mandate,
                fixture.manager,
                "Dominant Hall",
            ),
            (
                fixture.challenger,
                challenger_mandate,
                challenger_manager,
                "Challenger Corner",
            ),
        ] {
            let venue = insert_venue(
                &registry,
                &mut fixture.state,
                name,
                fixture.neighborhood,
                BusinessOwner::Organization(organization),
                hospitality_functions(),
            );
            establish_gambling_at_venue(
                &registry,
                &mut fixture.state,
                organization,
                mandate,
                manager,
                venue,
            );
        }

        let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
            .expect("influence should resolve");
        assert_eq!(
            summary.economic_leader(),
            None,
            "one racket each: contested"
        );
        for standing in &summary.standings {
            assert!(
                standing.governed,
                "both sides govern the contested district"
            );
        }
        validate_invariants(&fixture.state);
    }

    #[test]
    fn inactive_rackets_and_closed_venues_leave_the_footprint() {
        let registry = build_registry();
        let mut fixture = make_influence_fixture();

        let venue = insert_venue(
            &registry,
            &mut fixture.state,
            "Dominant Hall",
            fixture.neighborhood,
            BusinessOwner::Organization(fixture.dominant),
            hospitality_functions(),
        );
        let enterprise = establish_gambling_at_venue(
            &registry,
            &mut fixture.state,
            fixture.dominant,
            fixture.mandate,
            fixture.manager,
            venue,
        );
        validate_suspend_enterprise(&fixture.state, enterprise)
            .expect("suspension should validate")
            .commit(&mut fixture.state)
            .expect("suspension should commit");

        let summary = resolve_neighborhood_influence(&fixture.state, fixture.neighborhood)
            .expect("influence should resolve");
        let dominant = summary
            .standing_for(fixture.dominant)
            .expect("dominant standing should exist");
        assert_eq!(
            dominant.active_enterprises, 0,
            "a suspended racket projects no influence"
        );
        assert!(dominant.enterprise_kinds.is_empty());
        assert_eq!(dominant.owned_venues, 1);
        assert_eq!(summary.economic_leader(), None);
        validate_invariants(&fixture.state);
    }

    #[test]
    fn unknown_districts_are_rejected_without_state_change() {
        let registry = build_registry();
        let fixture = make_influence_fixture();
        let _ = registry;
        // An unallocated id of the right shape never resolves.
        let missing = crate::core::id::NeighborhoodId::from_raw(9_999);
        let error = match resolve_neighborhood_influence(&fixture.state, missing) {
            Err(error) => error,
            Ok(_) => panic!("unknown districts must be rejected"),
        };
        assert_eq!(error, TerritoryInfluenceError::MissingNeighborhood(missing));
    }
}
