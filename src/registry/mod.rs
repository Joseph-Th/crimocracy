//! Immutable code-owned registry: definition types, validated lookup tables, and assembly.
//!
//! Sibling files: `definitions.rs` owns the authored definition types; `builder.rs` owns
//! registration and validation; this module owns the `Registry` lookup surface.

mod builder;
mod definitions;

pub use definitions::*;

pub(crate) use builder::RegistryBuilder;

#[cfg(test)]
use crate::registry::builder::RegistryBuildError;

use crate::core::time::SimDuration;
use crate::enterprises::EnterpriseKind;
use crate::legal::InvestigationWorkKind;
use crate::operations::OperationKind;
use crate::world::{BusinessKind, PolicyKind, PolicySetting};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Registry {
    content_revision: u32,
    recruitment: RecruitmentDefinition,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    investigation_work: BTreeMap<InvestigationWorkKind, InvestigationWorkDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
    executive_brief: ExecutiveBriefDefinition,
    legal: LegalConfigDefinition,
    upkeep: UpkeepConfigDefinition,
    business_disruption: BusinessDisruptionDefinition,
    laundering: LaunderingConfigDefinition,
    reputation: ReputationConfigDefinition,
}

#[derive(Clone, Copy, Debug)]
pub struct LegalConfigDefinition {
    pub(super) cold_case_window: SimDuration,
    pub(super) witness_interview_attempt_limit: u8,
    pub(super) informant_decision_delay: SimDuration,
}

impl LegalConfigDefinition {
    /// How long an operation-originated investigation remains institutionally active after its
    /// last evidence/work activity before the owning authority deterministically shelves it.
    pub fn cold_case_window(self) -> SimDuration {
        self.cold_case_window
    }

    /// Completed interviews a case witness may sit through without producing a statement
    /// before investigators stop scheduling further futile interviews.
    pub fn witness_interview_attempt_limit(self) -> u8 {
        self.witness_interview_attempt_limit
    }

    /// How long after detention a detainee faces their single informant-recruitment decision.
    pub fn informant_decision_delay(self) -> SimDuration {
        self.informant_decision_delay
    }
}

impl Registry {
    pub fn content_revision(&self) -> u32 {
        self.content_revision
    }
    pub fn recruitment(&self) -> &RecruitmentDefinition {
        &self.recruitment
    }
    pub fn legal(&self) -> LegalConfigDefinition {
        self.legal
    }
    pub fn get_policy(&self, kind: PolicyKind) -> &PolicyDefinition {
        self.policies
            .get(&kind)
            .unwrap_or_else(|| panic!("missing policy definition: {kind:?}"))
    }
    pub fn get_operation(&self, kind: OperationKind) -> &OperationDefinition {
        self.operations
            .get(&kind)
            .unwrap_or_else(|| panic!("missing operation definition: {kind:?}"))
    }
    pub fn get_investigation_work(
        &self,
        kind: InvestigationWorkKind,
    ) -> &InvestigationWorkDefinition {
        self.investigation_work
            .get(&kind)
            .unwrap_or_else(|| panic!("missing investigation work definition: {kind:?}"))
    }
    pub fn get_enterprise(&self, kind: EnterpriseKind) -> &EnterpriseDefinition {
        self.enterprises
            .get(&kind)
            .unwrap_or_else(|| panic!("missing enterprise definition: {kind:?}"))
    }
    pub fn get_business(&self, kind: BusinessKind) -> &BusinessDefinition {
        self.businesses
            .get(&kind)
            .unwrap_or_else(|| panic!("missing business definition: {kind:?}"))
    }
    pub fn executive_brief(&self) -> ExecutiveBriefDefinition {
        self.executive_brief
    }
    pub fn upkeep(&self) -> UpkeepConfigDefinition {
        self.upkeep
    }
    pub fn business_disruption(&self) -> BusinessDisruptionDefinition {
        self.business_disruption
    }
    pub fn laundering(&self) -> LaunderingConfigDefinition {
        self.laundering
    }
    pub fn reputation(&self) -> ReputationConfigDefinition {
        self.reputation
    }
    pub(crate) fn default_policies(&self) -> BTreeMap<PolicyKind, PolicySetting> {
        self.policies
            .iter()
            .map(|(kind, def)| (*kind, def.default()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::AttentionClass;
    use crate::finance::Money;
    use crate::operations::{OperationApproach, RoleKind, ALL_OPERATION_KINDS};
    use crate::recruitment::RecruitmentApproach;
    use crate::world::{BusinessFunction, CapabilityKind, DriveKind, TraitKind};
    use std::collections::BTreeSet;

    fn burglary_operation_parts() -> (
        BTreeSet<OperationApproach>,
        BTreeSet<RoleKind>,
        OperationExecutionDefinition,
    ) {
        let registry = build_registry();
        let definition = registry.get_operation(OperationKind::Burglary);
        (
            definition.supported_approaches().clone(),
            definition.required_roles().clone(),
            definition.execution().clone(),
        )
    }

    #[test]
    fn operation_leaders_use_their_authored_domain_capability() {
        let registry = build_registry();

        assert_eq!(
            registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .leader_capability(),
            CapabilityKind::Management
        );
        assert_eq!(
            registry
                .get_operation(OperationKind::Surveillance)
                .execution()
                .leader_capability(),
            CapabilityKind::Surveillance
        );
    }

    fn recruitment_spec() -> RecruitmentDefinitionSpec {
        RecruitmentDefinitionSpec {
            timing: RecruitmentTimingDefinition {
                cooldown: SimDuration::from_minutes(60),
                autonomous_attempt_cadence: SimDuration::from_minutes(1_440),
                perceived_legal_pressure_max_age: SimDuration::from_minutes(1_440),
            },
            scoring: RecruitmentScoringDefinition {
                base_willingness: 10,
                acceptance_score: 40,
                existing_membership_resistance: 10,
                charismatic_recruiter_bonus: 5,
                weights: RecruitmentWeightsDefinition {
                    recruiter_influence: 25,
                    drive_alignment: 25,
                    relationship_support: 25,
                    incumbent_resentment: 10,
                    perceived_legal_pressure: 10,
                    incumbent_attachment: 25,
                    organization_competence: 10,
                },
            },
            recruiter_capabilities: BTreeSet::from([CapabilityKind::Negotiation]),
            relationships: RecruitmentRelationshipDefinition {
                recruiter_support: RecruitmentRelationshipSupportDefinition {
                    trust_weight: 1,
                    respect_weight: 1,
                    affection_weight: 1,
                    debt_weight: 1,
                    divisor: 4,
                    fear_penalty_weight: 1,
                    fear_penalty_divisor: 2,
                },
                incumbent_attachment: RecruitmentIncumbentRelationshipDefinition {
                    trust_weight: 1,
                    respect_weight: 1,
                    affection_weight: 1,
                    dependence_weight: 1,
                    divisor: 4,
                },
            },
            information_quality: RecruitmentInformationQualityDefinition {
                unknown_reliability: 20,
                unreliable_reliability: 10,
                mixed_reliability: 40,
                generally_reliable: 70,
                direct_access: 100,
                vague_specificity: 25,
                general_specificity: 50,
                specific_specificity: 75,
                precise_specificity: 100,
            },
            approach_drives: BTreeMap::from([
                (
                    RecruitmentApproach::FinancialOpportunity,
                    BTreeSet::from([DriveKind::Money]),
                ),
                (
                    RecruitmentApproach::Advancement,
                    BTreeSet::from([DriveKind::Status]),
                ),
                (
                    RecruitmentApproach::Protection,
                    BTreeSet::from([DriveKind::Safety]),
                ),
                (
                    RecruitmentApproach::PersonalAppeal,
                    BTreeSet::from([DriveKind::Respect]),
                ),
            ]),
            trait_rules: vec![RecruitmentTraitRuleDefinition {
                trait_kind: TraitKind::Ambitious,
                approach: Some(RecruitmentApproach::Advancement),
                minimum_incumbent_resentment: None,
                adjustment: 5,
            }],
        }
    }

    #[test]
    fn authored_recruitment_definition_is_complete_and_queryable() {
        let registry = build_registry();
        let definition = registry.recruitment();
        assert_eq!(definition.cooldown(), SimDuration::from_minutes(10_080));
        assert_eq!(
            definition.autonomous_attempt_cadence(),
            SimDuration::from_minutes(1_440)
        );
        assert_eq!(
            definition.recruiter_capabilities(),
            &BTreeSet::from([CapabilityKind::Negotiation, CapabilityKind::SocialAccess])
        );
        assert_eq!(
            definition.drives_for_approach(RecruitmentApproach::Protection),
            &BTreeSet::from([DriveKind::Safety, DriveKind::FamilySecurity])
        );
        assert!(definition
            .trait_rules()
            .iter()
            .any(|rule| rule.trait_kind == TraitKind::EasilyFrightened
                && rule.approach == Some(RecruitmentApproach::Protection)
                && rule.adjustment > 0));
    }

    #[test]
    fn authored_executive_brief_definition_is_bounded_and_queryable() {
        let definition = build_registry().executive_brief();
        assert_eq!(definition.cadence(), SimDuration::from_minutes(1_440));
        assert_eq!(
            definition.minimum_source_attention(),
            AttentionClass::Notable
        );
        assert_eq!(definition.max_source_entries(), 8);
    }

    #[test]
    fn authored_alcohol_distribution_requires_concrete_commercial_network() {
        let registry = build_registry();
        let definition = registry.get_enterprise(EnterpriseKind::AlcoholDistribution);
        assert!(definition.required_business_functions().is_empty());
        assert_eq!(
            definition.required_network_functions(),
            &BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
                BusinessFunction::CustomerAccess,
            ])
        );
        let economics = definition.economics();
        assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
        assert_eq!(economics.base_gross(), Money::from_cents(16_000));
        assert_eq!(economics.base_operating_cost(), Money::from_cents(10_000));
        assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(130));
        assert_eq!(
            economics.commerce_revenue_per_point(),
            Money::from_cents(50)
        );
        assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(25));
        assert_eq!(
            economics.management_revenue_per_point(),
            Money::from_cents(45)
        );
        assert_eq!(economics.police_cost_per_point(), Money::from_cents(40));
        assert_eq!(economics.gross_variance_basis_points(), 1_800);
        assert_eq!(economics.notable_variance_basis_points(), 1_200);
    }

    #[test]
    fn authored_police_response_definitions_are_bounded_and_queryable() {
        let registry = build_registry();
        let burglary = registry.get_operation(OperationKind::Burglary).execution();
        assert_eq!(burglary.police_dispatch_threshold(), 20);
        assert_eq!(
            burglary.base_police_response_delay(),
            SimDuration::from_minutes(12)
        );
        assert_eq!(
            burglary.minimum_police_response_delay(),
            SimDuration::from_minutes(3)
        );
        assert_eq!(burglary.patrol_response_reduction_minutes(), 9);
        assert_eq!(
            burglary.operation_entry_offset(),
            Some(SimDuration::from_minutes(10))
        );
        assert_eq!(burglary.police_arrival_difficulty_penalty(), 14);
        assert_eq!(burglary.police_arrival_exposure_penalty(), 18);

        let surveillance = registry
            .get_operation(OperationKind::Surveillance)
            .execution();
        assert_eq!(surveillance.operation_entry_offset(), None);
        assert!(surveillance.police_dispatch_threshold() > burglary.police_dispatch_threshold());
        for kind in ALL_OPERATION_KINDS {
            let response = registry.get_operation(kind).execution();
            assert!(response.police_dispatch_threshold() >= 0);
            assert!(response.police_dispatch_threshold() <= 100);
            assert!(response.minimum_police_response_delay().as_minutes() > 0);
            assert!(
                response.minimum_police_response_delay().as_minutes()
                    <= response.base_police_response_delay().as_minutes()
            );
            assert!(response.police_arrival_difficulty_penalty() <= 100);
            assert!(response.police_arrival_exposure_penalty() <= 100);
        }
    }

    #[test]
    fn operation_response_definition_rejects_invalid_timing_thresholds_and_penalties() {
        let (approaches, roles, execution) = burglary_operation_parts();
        let cases = [
            (
                {
                    let mut execution = execution.clone();
                    execution.police_response.dispatch_threshold = 101;
                    execution
                },
                RegistryBuildError::InvalidOperationResponseThreshold(OperationKind::Burglary),
            ),
            (
                {
                    let mut execution = execution.clone();
                    execution.police_response.minimum_response_delay = SimDuration::from_minutes(0);
                    execution
                },
                RegistryBuildError::InvalidOperationResponseDelay(OperationKind::Burglary),
            ),
            (
                {
                    let mut execution = execution.clone();
                    execution.police_response.patrol_reduction_minutes = 10;
                    execution
                },
                RegistryBuildError::InvalidOperationResponseReduction(OperationKind::Burglary),
            ),
            (
                {
                    let mut execution = execution.clone();
                    execution.police_response.entry_offset = Some(execution.duration());
                    execution
                },
                RegistryBuildError::InvalidOperationEntryOffset(OperationKind::Burglary),
            ),
            (
                {
                    let mut execution = execution.clone();
                    execution.police_response.arrival_exposure_penalty = 101;
                    execution
                },
                RegistryBuildError::InvalidOperationResponsePenalty(OperationKind::Burglary),
            ),
        ];
        for (execution, expected_error) in cases {
            let mut builder = RegistryBuilder::default();
            let error = builder
                .register_operation(
                    OperationKind::Burglary,
                    "Burglary",
                    approaches.clone(),
                    roles.clone(),
                    execution,
                )
                .expect_err("invalid police response authorship must be rejected");
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected_error)
            );
        }
    }

    #[test]
    fn executive_brief_definition_rejects_invalid_cadence_attention_and_entry_limit() {
        let valid = ExecutiveBriefDefinitionSpec {
            cadence: SimDuration::from_minutes(1_440),
            minimum_source_attention: AttentionClass::Notable,
            max_source_entries: 8,
        };

        let mut builder = RegistryBuilder::default();
        assert!(matches!(
            builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
                cadence: SimDuration::from_minutes(0),
                ..valid
            }),
            Err(RegistryBuildError::InvalidExecutiveBriefCadence)
        ));

        let mut builder = RegistryBuilder::default();
        assert!(matches!(
            builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
                minimum_source_attention: AttentionClass::Routine,
                ..valid
            }),
            Err(RegistryBuildError::InvalidExecutiveBriefAttention)
        ));

        for max_source_entries in [0, 101] {
            let mut builder = RegistryBuilder::default();
            assert!(matches!(
                builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
                    max_source_entries,
                    ..valid
                }),
                Err(RegistryBuildError::InvalidExecutiveBriefEntryLimit)
            ));
        }
    }

    #[test]
    fn recruitment_definition_rejects_zero_duration_and_incomplete_drive_mapping() {
        let mut builder = RegistryBuilder::default();
        let mut spec = recruitment_spec();
        spec.timing.cooldown = SimDuration::from_minutes(0);
        assert!(matches!(
            builder.register_recruitment(spec),
            Err(RegistryBuildError::InvalidRecruitmentDuration)
        ));

        let mut builder = RegistryBuilder::default();
        let mut spec = recruitment_spec();
        spec.approach_drives
            .remove(&RecruitmentApproach::Protection);
        assert!(matches!(
            builder.register_recruitment(spec),
            Err(RegistryBuildError::MissingRecruitmentApproachDrives(
                RecruitmentApproach::Protection
            ))
        ));
    }

    #[test]
    fn recruitment_definition_rejects_unsafe_relationship_math() {
        let mut builder = RegistryBuilder::default();
        let mut spec = recruitment_spec();
        spec.relationships.recruiter_support.divisor = 0;
        assert!(matches!(
            builder.register_recruitment(spec),
            Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights)
        ));

        let mut builder = RegistryBuilder::default();
        let mut spec = recruitment_spec();
        spec.relationships.recruiter_support.trust_weight = 5;
        spec.relationships.recruiter_support.divisor = 1;
        assert!(matches!(
            builder.register_recruitment(spec),
            Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights)
        ));
    }
}
