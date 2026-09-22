//! Matched treatment showing whether visible violence becomes future organizational leverage.
//! The acting policy never reads an exact reputation score: both branches run the same later
//! collection. Exact standing and resolution-factor comparisons below are harness evaluation.

use std::error::Error;

use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::BusinessId;
use crimocracy::core::time::SimDuration;
use crimocracy::economy::BusinessEconomyDraft;
use crimocracy::economy::business_economy_system::validate_establish_business_economy;
use crimocracy::finance::finance_system::insert_account;
use crimocracy::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::{
    OperationApproach, OperationConstraint, OperationDraft, OperationExposureLevel, OperationKind,
    OperationObjective, OperationObjectiveOutcome, RoleKind,
};
use crimocracy::registry::Registry;
use crimocracy::reputation::AudienceKind;
use crimocracy::reputation::reputation_system::resolve_score;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, build_scenario,
    run_until_operation_terminal, validate_harness_state,
};

#[derive(Clone, Debug)]
struct BranchEvidence {
    first_outcome: OperationObjectiveOutcome,
    first_exposure: OperationExposureLevel,
    business_fear: u8,
    second_outcome: OperationObjectiveOutcome,
    second_margin: i16,
    second_business_fear_adjustment: i8,
    second_summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViolenceLeverageSummary {
    pub visible_violence_created_fear: bool,
    pub fear_changed_later_intimidation: bool,
}

pub fn run_violence_leverage_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<ViolenceLeverageSummary, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let target = scenario.target;
    let alternate_target = scenario.alternate_target;
    establish_probe_business_economy(&mut scenario, target)?;
    establish_probe_business_economy(&mut scenario, alternate_target)?;
    let covert = run_branch(scenario.clone(), OperationApproach::Covert)?;
    let violent = run_branch(scenario, OperationApproach::Violent)?;
    let baseline = registry.reputation().baseline();

    let visible_violence_created_fear =
        violent.business_fear > baseline && covert.business_fear == baseline;
    let fear_changed_later_intimidation = violent.second_business_fear_adjustment < 0
        && covert.second_business_fear_adjustment == 0
        && violent.second_margin > covert.second_margin
        && violent.second_margin
            == covert.second_margin - i16::from(violent.second_business_fear_adjustment);
    if !visible_violence_created_fear || !fear_changed_later_intimidation {
        return Err(format!(
            "violence/reputation treatment lost its causal contrast: covert={covert:?}, violent={violent:?}"
        )
        .into());
    }

    if detail {
        println!(
            "[VIOLENCE] Matched arson treatment: covert ended {:?}/{:?}; visible violence ended {:?}/{:?}. Only the visible branch moved business-owner fear above its neutral standing.",
            covert.first_outcome,
            covert.first_exposure,
            violent.first_outcome,
            violent.first_exposure,
        );
        println!(
            "[FOLLOW-UP] The same opportunistic collection under a short deadline resolved {:?} at margin {} with neutral standing versus {:?} at margin {} after visible violence. The persisted fear adjustment improved the later margin by {} point(s). Whether that crosses an outcome threshold depends on the rest of the situation; no exact standing score informed the decision.",
            covert.second_outcome,
            covert.second_margin,
            violent.second_outcome,
            violent.second_margin,
            -violent.second_business_fear_adjustment,
        );
        println!("[LEARN] {}", violent.second_summary);
        println!(
            "[READ] Violence can buy bounded future compliance, but the first violent job also exposes the organization to the ordinary police/evidence/reputation consequences of a high-risk approach. Fear is leverage, not a universal success bonus."
        );
    }

    Ok(ViolenceLeverageSummary {
        visible_violence_created_fear,
        fear_changed_later_intimidation,
    })
}

fn establish_probe_business_economy(
    scenario: &mut Scenario<'_>,
    business: BusinessId,
) -> Result<(), Box<dyn Error>> {
    let operating_account = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )?;
    let settlement_account = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )?;
    validate_establish_business_economy(
        scenario.registry,
        &scenario.state,
        BusinessEconomyDraft {
            business,
            operating_account,
            settlement_account,
        },
    )?
    .commit(&mut scenario.state)?;
    Ok(())
}

fn run_branch(
    mut scenario: Scenario<'_>,
    first_approach: OperationApproach,
) -> Result<BranchEvidence, Box<dyn Error>> {
    let first = validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: format!("{first_approach:?} message at first venue"),
            kind: OperationKind::Arson,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::DisruptBusiness {
                target: EntityRef::Business(scenario.target),
            },
            approach: first_approach,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.lieutenant),
                (RoleKind::EntrySpecialist, scenario.burglar),
            ]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: scenario.state.now() + SimDuration::ONE_MINUTE,
        },
    )?
    .commit(&mut scenario.state)?;
    let mut metrics = RunMetrics::default();
    run_until_operation_terminal(&mut scenario, first, false, &mut metrics)?;
    let first_resolution = scenario
        .state
        .operations()
        .get_operation(first)
        .and_then(|record| record.resolution())
        .ok_or("violence leverage treatment arson did not resolve")?;
    let first_outcome = first_resolution.objective_outcome();
    let first_exposure = first_resolution.exposure().level();
    let business_fear = resolve_score(
        scenario.registry,
        scenario.state.reputation(),
        scenario.player,
        AudienceKind::Businesses,
    );

    let second_scheduled_for = scenario.state.now() + SimDuration::ONE_MINUTE;
    let second = validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: "Matched time-pressed follow-up collection".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(scenario.alternate_target),
            },
            approach: OperationApproach::Opportunistic,
            roles: BTreeMap::from([(RoleKind::Coordinator, scenario.lieutenant)]),
            intelligence: BTreeSet::new(),
            constraints: vec![OperationConstraint::CompleteBy(
                second_scheduled_for + SimDuration::from_minutes(14),
            )],
            contingencies: Vec::new(),
            scheduled_for: second_scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?;
    run_until_operation_terminal(&mut scenario, second, false, &mut metrics)?;
    let second_resolution = scenario
        .state
        .operations()
        .get_operation(second)
        .and_then(|record| record.resolution())
        .ok_or("violence leverage follow-up intimidation did not resolve")?;
    let second_outcome = second_resolution.objective_outcome();
    let second_margin = second_resolution.execution_margin();
    let second_business_fear_adjustment = second_resolution.factors().business_fear_adjustment();
    let second_summary = scenario
        .state
        .intelligence()
        .get_information(second_resolution.after_action_information())
        .ok_or("follow-up intimidation after-action information did not persist")?
        .summary()
        .to_owned();
    validate_harness_state(scenario.registry, &scenario.state)?;

    Ok(BranchEvidence {
        first_outcome,
        first_exposure,
        business_fear,
        second_outcome,
        second_margin,
        second_business_fear_adjustment,
        second_summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matched_visible_violence_changes_later_intimidation_through_business_fear() {
        let registry = crimocracy::build_registry();
        let summary =
            run_violence_leverage_probe(&registry, EvaluationSeeds::defaults(), false).unwrap();
        assert!(summary.visible_violence_created_fear);
        assert!(summary.fear_changed_later_intimidation);
    }
}
