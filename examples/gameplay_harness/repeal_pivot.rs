//! Repeal-pivot probe: post-Prohibition content through canonical production paths.
//!
//! The narrative arcs still play the classic gambling book, burglary, and surveillance
//! repertoire, so the authored narcotics/blackmail/graft rackets, the five newer fronts,
//! and the kidnapping/infiltration operations would otherwise have zero harness play.
//! This probe closes that divergence in one bounded scenario: it establishes one racket
//! of each new kind on fitting new-kind venues, settles a real enterprise cycle for each,
//! then runs one infiltration (the surveillance intel pipeline) and one kidnapping (the
//! largest cash take) to terminal through the canonical authorization path.
//! Acting policy uses only player-visible state; outcomes stay honest production results
//! rather than pinned successes. Operation approaches and targets vary on the policy seed
//! so repeated runs do not replay one exact treatment.

use crimocracy::core::entity::EntityRef;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::delegation::MandateAuthority;
use crimocracy::delegation::ResponsibilityScope;
use crimocracy::enterprises::enterprise_execution::validate_establish_enterprise;
use crimocracy::enterprises::{
    EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseStatus,
};
use crimocracy::finance::finance_system::insert_account;
use crimocracy::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::{
    OperationApproach, OperationContingency, OperationDraft, OperationKind, OperationObjective,
    OperationStatus, RoleKind,
};
use crimocracy::registry::Registry;
use crimocracy::world::world_system::insert_business;
use crimocracy::world::{BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use crate::*;

/// Distinct policy-seed salts so the operation treatments vary independently.
const INFILTRATION_APPROACH_SALT: u64 = 0xB16B_00B5;
const KIDNAPPING_APPROACH_SALT: u64 = 0x5EED_C0DE;
const INFILTRATION_TARGET_SALT: u64 = 0x0B5E_2A11;
const KIDNAPPING_TARGET_SALT: u64 = 0x2A15_0EED;

/// Policy-seeded approach treatment: covert, deceptive, or opportunistic. Inside assistance
/// is excluded because this probe staffs no inside contact; the three remaining postures
/// are all honest production authorizations for both operation kinds.
fn varied_probe_approach(policy_seed: u64, salt: u64) -> OperationApproach {
    match bounded_policy_choice(policy_seed, salt, 3) {
        0 => OperationApproach::Covert,
        1 => OperationApproach::Deceptive,
        _ => OperationApproach::Opportunistic,
    }
}

fn insert_probe_venue(
    scenario: &mut Scenario,
    name: &str,
    kind: BusinessKind,
    functions: &[BusinessFunction],
) -> Result<crimocracy::core::id::BusinessId, Box<dyn Error>> {
    Ok(insert_business(
        scenario.registry,
        &mut scenario.state,
        BusinessDraft {
            name: name.to_owned(),
            kind,
            functions: functions.iter().copied().collect(),
            neighborhood: scenario.neighborhood,
            owner: BusinessOwner::Organization(scenario.player),
        },
    )?)
}

fn establish_probe_enterprise(
    scenario: &mut Scenario,
    kind: EnterpriseKind,
    location: crimocracy::core::id::BusinessId,
    supporting: &[crimocracy::core::id::BusinessId],
) -> Result<crimocracy::core::id::EnterpriseId, Box<dyn Error>> {
    let cash = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(scenario.player),
            kind: AccountKind::StreetCash,
        },
    )?;
    let settlement = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(scenario.player),
            kind: AccountKind::Settlement,
        },
    )?;
    Ok(validate_establish_enterprise(
        scenario.registry,
        &scenario.state,
        EnterpriseDraft {
            kind,
            organization: scenario.player,
            authority: MandateAuthority {
                mandate: scenario.lieutenant_mandate,
                manager: scenario.lieutenant,
                scope: ResponsibilityScope::Neighborhood(scenario.neighborhood),
            },
            location: EnterpriseLocation::Business(location),
            supporting_businesses: supporting.iter().copied().collect(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )?
    .commit(&mut scenario.state)?)
}

/// Proves the repeal-pivot content is playable through production paths: three new-kind
/// rackets on five new-kind fronts settle real cycles, and both new operation kinds reach
/// terminal state. New venues sit in the home district under the existing lieutenant
/// mandate, and the district starts clean so no hidden casework confounds the cycles.
pub fn run_repeal_pivot_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Recon),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };

    // Narcotics: pharmacy host covers cash handling, customer access, and prescription
    // records; the taxi garage contributes the distribution infrastructure neither covers.
    let pharmacy = insert_probe_venue(
        &mut scenario,
        "Canal Pharmacy",
        BusinessKind::Pharmacy,
        &[
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::ProfessionalRecords,
        ],
    )?;
    let taxi_garage = insert_probe_venue(
        &mut scenario,
        "Canal Taxi Garage",
        BusinessKind::TaxiGarage,
        &[
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::VehicleFleet,
            BusinessFunction::VehicleWorkshop,
        ],
    )?;
    // Blackmail: the funeral parlor hosts records and meetings; the movie house draws the
    // crowd while the lodging house boards the mark. Each support uniquely covers one
    // network function so the minimal-support rule holds.
    let funeral_parlor = insert_probe_venue(
        &mut scenario,
        "Silent Rest Funeral Parlor",
        BusinessKind::FuneralParlor,
        &[
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::MeetingSpace,
        ],
    )?;
    let movie_house = insert_probe_venue(
        &mut scenario,
        "Rialto Movie House",
        BusinessKind::MovieTheater,
        &[BusinessFunction::CustomerAccess],
    )?;
    let lodging = insert_probe_venue(
        &mut scenario,
        "Harbor Lodging House",
        BusinessKind::Lodging,
        &[BusinessFunction::Lodging],
    )?;
    // Municipal graft: the soup kitchen hosts while the contractor carries union access
    // and the financial office carries money movement plus its own books.
    let soup_kitchen = insert_probe_venue(
        &mut scenario,
        "Breadline Soup Kitchen",
        BusinessKind::SoupKitchen,
        &[
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ],
    )?;
    let contractor = insert_probe_venue(
        &mut scenario,
        "Pierworks Construction",
        BusinessKind::Construction,
        &[BusinessFunction::UnionAccess],
    )?;
    let financial_office = insert_probe_venue(
        &mut scenario,
        "Ledger and Trust Office",
        BusinessKind::FinancialServices,
        &[
            BusinessFunction::FinancialServices,
            BusinessFunction::ProfessionalRecords,
        ],
    )?;

    let narcotics = establish_probe_enterprise(
        &mut scenario,
        EnterpriseKind::NarcoticsTrade,
        pharmacy,
        &[taxi_garage],
    )?;
    let blackmail = establish_probe_enterprise(
        &mut scenario,
        EnterpriseKind::Blackmail,
        funeral_parlor,
        &[movie_house, lodging],
    )?;
    let graft = establish_probe_enterprise(
        &mut scenario,
        EnterpriseKind::MunicipalGraft,
        soup_kitchen,
        &[contractor, financial_office],
    )?;
    validate_harness_state(registry, &scenario.state)?;

    // Settle one real cycle for every new book. All three share the authored daily cycle,
    // so a single wait to the latest first-cycle minute covers the cohort.
    let first_cycles = [narcotics, blackmail, graft]
        .iter()
        .map(|enterprise| {
            scenario
                .state
                .enterprises()
                .get_enterprise(*enterprise)
                .and_then(|record| record.next_cycle_at())
                .ok_or("pivot enterprise must schedule its first cycle")
        })
        .collect::<Result<Vec<SimTime>, &str>>()?;
    let cohort_due = first_cycles
        .into_iter()
        .max()
        .expect("three enterprises were built");
    run_until(&mut scenario, cohort_due, false, &mut metrics)?;
    for enterprise in [narcotics, blackmail, graft] {
        let settled = scenario.state.enterprises().cycles_for(enterprise).count();
        if settled == 0 {
            return Err(format!(
                "repeal-pivot racket {enterprise:?} settled no cycle by the cohort boundary"
            )
            .into());
        }
        let record = scenario
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("established pivot enterprise must persist");
        if record.status() != EnterpriseStatus::Active {
            return Err(format!(
                "repeal-pivot racket {enterprise:?} did not stay active through its first cycle"
            )
            .into());
        }
    }
    if detail {
        println!(
            "[PIVOT] Narcotics, blackmail, and graft books each settled a cycle on new-kind fronts; all three remain active."
        );
    }

    // Infiltration: plant the scout inside a rival organization through the canonical
    // surveillance intel pipeline. Both rivals are organization-visible, so the
    // policy-seeded choice between them uses no hidden knowledge.
    let infiltration_approach = varied_probe_approach(seeds.policy, INFILTRATION_APPROACH_SALT);
    let infiltrated_rival = if bounded_policy_choice(seeds.policy, INFILTRATION_TARGET_SALT, 2) == 0
    {
        scenario.rival
    } else {
        scenario.second_rival
    };
    let infiltrated_name = scenario
        .state
        .world()
        .get_organization(infiltrated_rival)
        .expect("probe rival must persist")
        .name()
        .to_owned();
    let infiltration = validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: format!("Place Mara inside the {infiltrated_name}"),
            kind: OperationKind::Infiltration,
            responsible_organization: scenario.player,
            leader: scenario.scout,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Organization(infiltrated_rival),
            },
            approach: infiltration_approach,
            roles: BTreeMap::from([(RoleKind::Coordinator, scenario.scout)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: scenario.state.now() + SimDuration::ONE_MINUTE,
        },
    )?
    .commit(&mut scenario.state)?;
    run_until_operation_terminal(&mut scenario, infiltration, false, &mut metrics)?;
    let infiltration_record = scenario
        .state
        .operations()
        .get_operation(infiltration)
        .expect("authorized infiltration must remain queryable");
    if !matches!(
        infiltration_record.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return Err("infiltration did not reach terminal state".into());
    }
    let infiltration_findings = infiltration_record
        .resolution()
        .map(|resolution| resolution.discovered_information().len())
        .unwrap_or_default();
    if detail {
        println!(
            "[PIVOT] Infiltration ({infiltration_approach:?}) reached {:?} with {infiltration_findings} discovered finding(s) through the surveillance pipeline.",
            infiltration_record.status(),
        );
    }

    // Kidnapping: ransom a foreign family business. Both the character-owned score and
    // the independent annex are foreign Retail targets with no venue-function gate, so the
    // policy-seeded choice between them stays a valid production authorization either way.
    // The lieutenant coordinates while the entry specialist drives; any terminal outcome
    // is honest production evidence rather than a pinned success.
    let kidnapping_approach = varied_probe_approach(seeds.policy, KIDNAPPING_APPROACH_SALT);
    let ransom_target = if bounded_policy_choice(seeds.policy, KIDNAPPING_TARGET_SALT, 2) == 0 {
        scenario.alternate_target
    } else {
        scenario.target
    };
    let ransom_name = scenario
        .state
        .world()
        .get_business(ransom_target)
        .expect("probe ransom target must persist")
        .name()
        .to_owned();
    let kidnapping = validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: format!("Seize the {ransom_name} family for ransom"),
            kind: OperationKind::Kidnapping,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(ransom_target),
            },
            approach: kidnapping_approach,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.lieutenant),
                (RoleKind::Driver, scenario.burglar),
            ]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: vec![OperationContingency::RequestDecisionOnPoliceArrival],
            scheduled_for: scenario.state.now() + SimDuration::ONE_MINUTE,
        },
    )?
    .commit(&mut scenario.state)?;
    run_until_operation_terminal(&mut scenario, kidnapping, false, &mut metrics)?;
    let kidnapping_record = scenario
        .state
        .operations()
        .get_operation(kidnapping)
        .expect("authorized kidnapping must remain queryable");
    if !matches!(
        kidnapping_record.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return Err("kidnapping did not reach terminal state".into());
    }
    if detail {
        println!(
            "[PIVOT] Kidnapping ({kidnapping_approach:?}) reached {:?} with outcome {:?}; the ransom take path resolves through production economics.",
            kidnapping_record.status(),
            kidnapping_record
                .resolution()
                .map(|resolution| resolution.objective_outcome()),
        );
    }

    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[PIVOT PASS] post-repeal content plays through canonical paths: three new rackets, five new fronts, infiltration, kidnapping."
        );
    }
    Ok(())
}
