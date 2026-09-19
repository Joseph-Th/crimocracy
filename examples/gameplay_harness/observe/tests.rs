//! Decision ownership contracts through canonical requests and resolution.

use super::*;
use crimocracy::core::simulation::run_tick;
use crimocracy::decisions::decision_system::validate_request_recruitment_approval;
use crimocracy::decisions::{DecisionStatus, RecruitmentApprovalRequestDraft};
use crimocracy::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};
use crimocracy::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crimocracy::recruitment::RecruitmentApproach;
use crimocracy::world::{ApprovalPolicy, PolicyKind, PolicySetting};

fn approval_request(
    scenario: &mut Scenario,
    player_owned: bool,
) -> crimocracy::decisions::decision_system::DecisionRequestOutcome {
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let organization = if player_owned {
        scenario.player
    } else {
        scenario.rival
    };
    let mandate = if player_owned {
        scenario
            .state
            .delegation()
            .get_mandate(scenario.lieutenant_mandate)
            .unwrap()
    } else {
        scenario
            .state
            .delegation()
            .active_for_scope(scope)
            .find(|item| item.organization() == organization)
            .unwrap()
    };
    let id = mandate.id();
    let manager = mandate.manager();
    let mut scopes = mandate.scopes().clone();
    scopes.insert(scope);
    let mut orders = mandate.standing_orders().clone();
    orders.insert(
        PolicyKind::IndependentRecruitment,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    );
    validate_revise_mandate(
        &scenario.state,
        id,
        MandateRevisionDraft {
            scopes,
            standing_orders: orders,
            budget: mandate.budget(),
        },
    )
    .unwrap()
    .commit(&mut scenario.state)
    .unwrap();
    if player_owned {
        use crimocracy::social::{RelationshipDimensions, RelationshipLevel};
        let known = RelationshipLevel::try_new(50).unwrap();
        crimocracy::social::relationship_system::validate_set_relationship(
            &scenario.state,
            scenario.danny_ferro,
            manager,
            RelationshipDimensions {
                trust: known,
                respect: known,
                fear: known,
                affection: known,
                dependence: known,
                resentment: RelationshipLevel::try_new(0).unwrap(),
                debt: known,
            },
        )
        .unwrap()
        .commit(&mut scenario.state)
        .unwrap();
    }
    validate_request_recruitment_approval(
        scenario.registry,
        &scenario.state,
        RecruitmentApprovalRequestDraft {
            authority: MandateAuthority {
                mandate: id,
                manager,
                scope,
            },
            target_organization: organization,
            recruiter: manager,
            candidate: if player_owned {
                scenario.danny_ferro
            } else {
                scenario.burglar
            },
            approach: RecruitmentApproach::Protection,
            attention: AttentionClass::Exception,
            summary: "Manager requests a recruitment decision.".to_owned(),
        },
    )
    .unwrap()
    .commit(&mut scenario.state)
    .unwrap()
}

#[test]
fn foreign_pending_and_resolved_requests_do_not_become_player_decisions() {
    let registry = crimocracy::build_registry();
    for resolved in [false, true] {
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let mut tick = run_tick(&registry, &mut scenario.state).unwrap();
        let request = approval_request(&mut scenario, false);
        if resolved {
            validate_resolve_decision(
                &registry,
                &scenario.state,
                request.decision,
                scenario.rival,
                DecisionResponse::Reject,
            )
            .unwrap()
            .commit(&mut scenario.state)
            .unwrap();
        }
        tick.decision_requests.push(request);
        let before = bincode::serialize(&scenario.state).unwrap();
        let mut metrics = RunMetrics::default();
        resolve_decision_requests(&mut scenario, &tick, false, &mut metrics).unwrap();
        assert_eq!(metrics.decision_requests, 0);
        assert_eq!(bincode::serialize(&scenario.state).unwrap(), before);
    }
}

#[test]
fn player_request_is_rejected_once_and_terminal_reobservation_is_inert() {
    let registry = crimocracy::build_registry();
    let mut scenario = build_scenario(
        &registry,
        EvaluationSeeds::defaults(),
        ScenarioProfile::NightTrap,
    )
    .unwrap();
    let mut tick = run_tick(&registry, &mut scenario.state).unwrap();
    let request = approval_request(&mut scenario, true);
    let id = request.decision;
    tick.decision_requests.push(request);
    let mut metrics = RunMetrics::default();
    resolve_decision_requests(&mut scenario, &tick, false, &mut metrics).unwrap();
    let decision = scenario.state.decisions().get_decision(id).unwrap();
    assert_eq!(decision.status(), DecisionStatus::Resolved);
    assert_eq!(
        decision.resolution().unwrap().resolved_by(),
        scenario.player
    );
    assert_eq!(
        decision.resolution().unwrap().response(),
        DecisionResponse::Reject
    );
    assert_eq!(metrics.decision_requests, 1);
    let before = bincode::serialize(&scenario.state).unwrap();
    resolve_decision_requests(&mut scenario, &tick, false, &mut metrics).unwrap();
    assert_eq!(metrics.decision_requests, 1);
    assert_eq!(bincode::serialize(&scenario.state).unwrap(), before);
}
