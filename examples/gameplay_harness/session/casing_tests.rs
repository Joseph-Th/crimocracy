//! Canonical scout outcomes, not just metric fixtures, protect the shared casing boundary.
use super::*;
use crimocracy::operations::operation_system::{OperationTransition, apply_transition};

#[test]
fn aborted_canonical_scout_is_not_read_as_a_completed_resolution() {
    let registry = crimocracy::build_registry();
    let mut scenario = build_scenario(
        &registry,
        EvaluationSeeds::defaults(),
        ScenarioProfile::NightTrap,
    )
    .unwrap();
    let scout = authorize_surveillance(&mut scenario).unwrap();
    apply_transition(
        &registry,
        &mut scenario.state,
        scout,
        OperationTransition::Abort,
    )
    .unwrap();
    assert!(
        scenario
            .state
            .operations()
            .get_operation(scout)
            .unwrap()
            .resolution()
            .is_none()
    );
    let before = bincode::serialize(&scenario.state).unwrap();
    let mut metrics = RunMetrics::default();
    let assessment = assess_casing(&mut scenario, scout, false, &mut metrics).unwrap();
    assert_eq!(assessment, CasingAssessment::Aborted);
    assert!(!assessment.permits_burglary());
    assert_eq!(metrics.contact_reads, 0);
    assert_eq!(bincode::serialize(&scenario.state).unwrap(), before);
}

#[test]
fn active_opening_case_withholds_both_scores_without_fabricated_burglary() {
    let registry = crimocracy::build_registry();
    let metrics = play_session(
        &registry,
        Strategy::Recon,
        ScenarioProfile::NightTrap,
        EvaluationSeeds::new(0x19330517, DEFAULT_POLICY_SEED),
        SessionRunMode::FullQuiet,
    )
    .unwrap();
    assert_eq!(
        metrics.opening_casing_assessment,
        Some(CasingAssessment::Active)
    );
    assert!(metrics.opening_stood_down);
    assert!(metrics.opening_scout.is_some());
    assert_eq!(metrics.contact_reads, 1);
    assert_eq!(metrics.burglary, None);
    assert_eq!(metrics.outcome, None);
    assert!(!metrics.aborted);
    assert_eq!(metrics.second_scout, None);
    assert_eq!(metrics.second_burglary, None);
    assert!(metrics.second_opportunity_expired);
    assert_eq!(metrics.property_realized_cash_cents, None);
    validate_run_metrics(&metrics, true).unwrap();
    validate_second_act_evidence(&metrics).unwrap();
}
