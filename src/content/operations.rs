//! Authored semantic-operation approaches, roles, execution, exposure, policing, and proceeds.

use crate::core::time::SimDuration;
use crate::intelligence::InformationTopic;
use crate::legal::EvidenceKind;
use crate::operations::{ALL_OPERATION_KINDS, OperationApproach, OperationKind, RoleKind};
use crate::registry::{
    OperationBusinessTargetDefinition, OperationCashProceedsDefinition,
    OperationDifficultyDefinition, OperationExecutionDefinition, OperationExposureDefinition,
    OperationIntelligenceDefinition, OperationPoliceResponseDefinition,
    OperationPropertyProceedsDefinition, RegistryBuilder,
};
use crate::world::CapabilityKind;
use std::collections::BTreeSet;

const MINIMUM_POLICE_RESPONSE_DELAY_MINUTES: u32 = 3;
const RECENT_TAKE_RECOVERY_WINDOW: SimDuration = SimDuration::from_minutes(3 * 24 * 60);
const IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS: u16 = 5_000;
const LIQUIDATION_POLICE_NEUTRAL_RATING: u8 = 50;
const LIQUIDATION_POLICE_ADJUSTMENT_BASIS_POINTS_PER_POINT: u16 = 20;
const LIQUIDATION_MIN_RECOVERY_BASIS_POINTS: u16 = 3_000;
const LIQUIDATION_MAX_RECOVERY_BASIS_POINTS: u16 = 9_000;

pub(super) fn register_operations(builder: &mut RegistryBuilder) {
    for kind in ALL_OPERATION_KINDS {
        let approaches = supported_operation_approaches(kind)
            .iter()
            .copied()
            .collect();
        let roles = required_roles(kind);
        builder
            .register_operation(
                kind,
                operation_name(kind),
                approaches,
                roles.clone(),
                operation_execution(kind),
            )
            .unwrap_or_else(|error| panic!("invalid operation registry: {error}"));
    }
}

fn operation_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Burglary => "Burglary",
        OperationKind::Robbery => "Robbery",
        OperationKind::Hijacking => "Hijacking",
        OperationKind::Smuggling => "Smuggling",
        OperationKind::Intimidation => "Intimidation",
        OperationKind::Surveillance => "Surveillance",
        OperationKind::WitnessPressure => "Witness pressure",
        OperationKind::DocumentTheft => "Document theft",
        OperationKind::GamblingEvent => "Gambling event",
        OperationKind::Extraction => "Extraction",
        OperationKind::Sabotage => "Sabotage",
        OperationKind::Arson => "Arson",
    }
}

fn supported_operation_approaches(kind: OperationKind) -> &'static [OperationApproach] {
    match kind {
        OperationKind::Burglary | OperationKind::DocumentTheft => &[
            OperationApproach::Covert,
            OperationApproach::Deceptive,
            OperationApproach::InsideAssistance,
            OperationApproach::Opportunistic,
        ],
        OperationKind::Robbery => &[
            OperationApproach::Deceptive,
            OperationApproach::Intimidating,
            OperationApproach::Violent,
            OperationApproach::InsideAssistance,
            OperationApproach::Opportunistic,
        ],
        OperationKind::Hijacking
        | OperationKind::Intimidation
        | OperationKind::WitnessPressure
        | OperationKind::Extraction => &[
            OperationApproach::Covert,
            OperationApproach::Deceptive,
            OperationApproach::Intimidating,
            OperationApproach::Violent,
            OperationApproach::InsideAssistance,
            OperationApproach::Opportunistic,
        ],
        OperationKind::Smuggling | OperationKind::Surveillance | OperationKind::GamblingEvent => &[
            OperationApproach::Covert,
            OperationApproach::Deceptive,
            OperationApproach::InsideAssistance,
            OperationApproach::Opportunistic,
        ],
        OperationKind::Sabotage | OperationKind::Arson => &[
            OperationApproach::Covert,
            OperationApproach::Deceptive,
            OperationApproach::Violent,
            OperationApproach::InsideAssistance,
            OperationApproach::Opportunistic,
        ],
    }
}

fn required_roles(kind: OperationKind) -> BTreeSet<RoleKind> {
    let roles: &[RoleKind] = match kind {
        OperationKind::Burglary => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::Robbery => &[RoleKind::Coordinator, RoleKind::Muscle],
        OperationKind::Hijacking => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Smuggling => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Intimidation => &[RoleKind::Coordinator],
        OperationKind::Surveillance => &[RoleKind::Surveillance],
        OperationKind::WitnessPressure => &[RoleKind::Coordinator],
        OperationKind::DocumentTheft => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::GamblingEvent => &[RoleKind::Coordinator],
        OperationKind::Extraction => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Sabotage => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::Arson => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
    };
    roles.iter().copied().collect()
}

fn supported_operation_roles(kind: OperationKind) -> &'static [RoleKind] {
    match kind {
        OperationKind::Burglary => &[
            RoleKind::Coordinator,
            RoleKind::EntrySpecialist,
            RoleKind::SafeSpecialist,
            RoleKind::Lookout,
            RoleKind::Driver,
            RoleKind::InsideContact,
        ],
        OperationKind::Robbery => &[
            RoleKind::Coordinator,
            RoleKind::Muscle,
            RoleKind::Driver,
            RoleKind::Lookout,
            RoleKind::SafeSpecialist,
            RoleKind::InsideContact,
        ],
        OperationKind::Hijacking => &[
            RoleKind::Coordinator,
            RoleKind::Driver,
            RoleKind::Lookout,
            RoleKind::Muscle,
            RoleKind::InsideContact,
        ],
        OperationKind::Smuggling => &[
            RoleKind::Coordinator,
            RoleKind::Driver,
            RoleKind::Lookout,
            RoleKind::InsideContact,
            RoleKind::Negotiator,
        ],
        OperationKind::Intimidation | OperationKind::WitnessPressure => &[
            RoleKind::Coordinator,
            RoleKind::Muscle,
            RoleKind::Negotiator,
            RoleKind::InsideContact,
        ],
        OperationKind::Surveillance => &[
            RoleKind::Surveillance,
            RoleKind::Lookout,
            RoleKind::Driver,
            RoleKind::InsideContact,
            RoleKind::Coordinator,
        ],
        OperationKind::DocumentTheft => &[
            RoleKind::Coordinator,
            RoleKind::EntrySpecialist,
            RoleKind::SafeSpecialist,
            RoleKind::Lookout,
            RoleKind::Driver,
            RoleKind::InsideContact,
        ],
        OperationKind::GamblingEvent => &[
            RoleKind::Coordinator,
            RoleKind::Negotiator,
            RoleKind::InsideContact,
            RoleKind::Lookout,
            RoleKind::Muscle,
        ],
        OperationKind::Extraction => &[
            RoleKind::Coordinator,
            RoleKind::Driver,
            RoleKind::Muscle,
            RoleKind::Lookout,
            RoleKind::InsideContact,
            RoleKind::EntrySpecialist,
        ],
        OperationKind::Sabotage | OperationKind::Arson => &[
            RoleKind::Coordinator,
            RoleKind::EntrySpecialist,
            RoleKind::Driver,
            RoleKind::Lookout,
            RoleKind::InsideContact,
        ],
    }
}

fn operation_execution(kind: OperationKind) -> OperationExecutionDefinition {
    let (duration_minutes, base_difficulty, police_pressure_weight, base_exposure) = match kind {
        OperationKind::Burglary => (45, 52, 45, 38),
        OperationKind::Robbery => (25, 55, 55, 58),
        OperationKind::Hijacking => (35, 50, 45, 48),
        OperationKind::Smuggling => (90, 48, 35, 35),
        OperationKind::Intimidation => (20, 42, 25, 42),
        OperationKind::Surveillance => (120, 40, 20, 50),
        OperationKind::WitnessPressure => (30, 50, 35, 48),
        OperationKind::DocumentTheft => (30, 50, 40, 36),
        OperationKind::GamblingEvent => (180, 38, 30, 45),
        OperationKind::Extraction => (60, 58, 50, 52),
        // Sabotage is deliberate property damage: quieter than robbery, slower than
        // intimidation, and heavily dependent on knowing the target's layout.
        OperationKind::Sabotage => (55, 48, 30, 40),
        // Arson is high-risk message violence: short, exposed, and heavily penalized
        // by police presence — the blunt counterpart to sabotage.
        OperationKind::Arson => (30, 60, 55, 62),
    };
    let role_capabilities = supported_operation_roles(kind)
        .iter()
        .copied()
        .map(|role| (role, capability_for_operation_role(role)))
        .collect();
    let leader_capability = match kind {
        OperationKind::Surveillance => CapabilityKind::Surveillance,
        OperationKind::WitnessPressure => CapabilityKind::Intimidation,
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Extraction
        | OperationKind::Sabotage
        | OperationKind::Arson => CapabilityKind::Management,
    };
    let approach_difficulty_adjustments = supported_operation_approaches(kind)
        .iter()
        .copied()
        .map(|approach| {
            let adjustment = match approach {
                OperationApproach::Covert => -5,
                OperationApproach::Deceptive => -2,
                OperationApproach::Intimidating => 3,
                OperationApproach::Violent => 6,
                OperationApproach::InsideAssistance => -8,
                OperationApproach::Opportunistic => 4,
            };
            (approach, adjustment)
        })
        .collect();
    let exposure_approach_adjustments = supported_operation_approaches(kind)
        .iter()
        .copied()
        .map(|approach| {
            let adjustment = match approach {
                OperationApproach::Covert => -12,
                OperationApproach::Deceptive => -5,
                OperationApproach::Intimidating => 10,
                OperationApproach::Violent => 18,
                OperationApproach::InsideAssistance => -10,
                OperationApproach::Opportunistic => 6,
            };
            (approach, adjustment)
        })
        .collect();
    let (
        dispatch_threshold,
        base_response_minutes,
        entry_minutes,
        response_difficulty,
        response_exposure,
    ) = match kind {
        OperationKind::Burglary => (20, 12, Some(10), 14, 18),
        OperationKind::Robbery => (12, 8, Some(6), 18, 24),
        OperationKind::Hijacking => (18, 10, Some(5), 16, 22),
        OperationKind::Smuggling => (28, 15, None, 12, 18),
        OperationKind::Intimidation => (24, 10, None, 12, 18),
        OperationKind::Surveillance => (45, 18, None, 10, 14),
        OperationKind::WitnessPressure => (24, 12, None, 14, 20),
        OperationKind::DocumentTheft => (20, 12, Some(8), 14, 18),
        OperationKind::GamblingEvent => (35, 15, None, 10, 16),
        OperationKind::Extraction => (18, 10, Some(8), 18, 24),
        OperationKind::Sabotage => (24, 12, Some(8), 14, 20),
        OperationKind::Arson => (14, 8, Some(5), 20, 26),
    };
    OperationExecutionDefinition {
        difficulty: OperationDifficultyDefinition {
            duration: SimDuration::from_minutes(duration_minutes),
            base_difficulty,
            role_capabilities,
            // Execution is primarily crew-role skill, with leadership contributing one quarter
            // of the effective ability under stock content. Keep the ratio authored so balance
            // changes do not require changing resolution code.
            role_capability_weight: 3,
            leader_capability_weight: 1,
            approach_difficulty_adjustments,
            police_pressure_weight,
            max_time_pressure: 30,
            variance_limit: 12,
            achieved_margin: 5,
            partial_margin: -12,
        },
        leader_capability,
        business_target: match kind {
            OperationKind::GamblingEvent => Some(OperationBusinessTargetDefinition {
                required_functions: super::businesses::gambling_venue_functions(),
            }),
            OperationKind::Burglary
            | OperationKind::Robbery
            | OperationKind::Hijacking
            | OperationKind::Smuggling
            | OperationKind::Intimidation
            | OperationKind::DocumentTheft
            | OperationKind::Sabotage
            | OperationKind::Arson => Some(OperationBusinessTargetDefinition {
                required_functions: BTreeSet::new(),
            }),
            OperationKind::Surveillance
            | OperationKind::WitnessPressure
            | OperationKind::Extraction => None,
        },
        intelligence: OperationIntelligenceDefinition {
            relevant_topics: relevant_operation_intelligence(kind),
            max_difficulty_reduction: 14,
            max_useful_age: SimDuration::from_minutes(10_080),
            patrol_observation_bucket: SimDuration::from_minutes(30),
        },
        exposure: OperationExposureDefinition {
            base_exposure,
            approach_adjustments: exposure_approach_adjustments,
            police_observation_weight: 35,
            stealth_mitigation_weight: 45,
            intelligence_mitigation_weight: 20,
            variance_limit: 12,
            trace_threshold: 20,
            witnessed_threshold: 45,
            identifying_threshold: 65,
            witness_reluctant_police_presence: 30,
            witness_cooperative_police_presence: 60,
            high_police_presence_narrative_threshold: 65,
            evidence_kind: operation_exposure_evidence_kind(kind),
        },
        police_response: OperationPoliceResponseDefinition {
            dispatch_threshold,
            base_response_delay: SimDuration::from_minutes(base_response_minutes),
            minimum_response_delay: SimDuration::from_minutes(
                MINIMUM_POLICE_RESPONSE_DELAY_MINUTES,
            ),
            patrol_reduction_minutes: u16::try_from(
                base_response_minutes - MINIMUM_POLICE_RESPONSE_DELAY_MINUTES,
            )
            .expect("authored police response delay range must fit u16"),
            entry_offset: entry_minutes.map(SimDuration::from_minutes),
            arrival_difficulty_penalty: response_difficulty,
            arrival_exposure_penalty: response_exposure,
        },
        property_proceeds: match kind {
            OperationKind::Burglary => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 30_000,
                partial_recovery_basis_points: 4_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
                liquidation_recovery_basis_points: 6_500,
                liquidation_police_neutral_rating: LIQUIDATION_POLICE_NEUTRAL_RATING,
                liquidation_police_adjustment_basis_points_per_point:
                    LIQUIDATION_POLICE_ADJUSTMENT_BASIS_POINTS_PER_POINT,
                liquidation_min_recovery_basis_points: LIQUIDATION_MIN_RECOVERY_BASIS_POINTS,
                liquidation_max_recovery_basis_points: LIQUIDATION_MAX_RECOVERY_BASIS_POINTS,
            }),
            OperationKind::Hijacking => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 25_000,
                partial_recovery_basis_points: 3_500,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
                liquidation_recovery_basis_points: 5_500,
                liquidation_police_neutral_rating: LIQUIDATION_POLICE_NEUTRAL_RATING,
                liquidation_police_adjustment_basis_points_per_point:
                    LIQUIDATION_POLICE_ADJUSTMENT_BASIS_POINTS_PER_POINT,
                liquidation_min_recovery_basis_points: LIQUIDATION_MIN_RECOVERY_BASIS_POINTS,
                liquidation_max_recovery_basis_points: LIQUIDATION_MAX_RECOVERY_BASIS_POINTS,
            }),
            OperationKind::DocumentTheft => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 12_500,
                partial_recovery_basis_points: 5_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
                liquidation_recovery_basis_points: 4_000,
                liquidation_police_neutral_rating: LIQUIDATION_POLICE_NEUTRAL_RATING,
                liquidation_police_adjustment_basis_points_per_point:
                    LIQUIDATION_POLICE_ADJUSTMENT_BASIS_POINTS_PER_POINT,
                liquidation_min_recovery_basis_points: LIQUIDATION_MIN_RECOVERY_BASIS_POINTS,
                liquidation_max_recovery_basis_points: LIQUIDATION_MAX_RECOVERY_BASIS_POINTS,
            }),
            OperationKind::Robbery
            | OperationKind::Smuggling
            | OperationKind::Intimidation
            | OperationKind::Surveillance
            | OperationKind::WitnessPressure
            | OperationKind::GamblingEvent
            | OperationKind::Extraction
            | OperationKind::Sabotage
            | OperationKind::Arson => None,
        },
        cash_proceeds: match kind {
            // Robbery takes the till directly; intimidation collects protection money;
            // a gambling event keeps the house edge; a smuggling run is paid on delivery.
            OperationKind::Robbery => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 40_000,
                partial_take_basis_points: 8_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
            }),
            OperationKind::Intimidation => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 15_000,
                partial_take_basis_points: 3_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
            }),
            OperationKind::GamblingEvent => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 20_000,
                partial_take_basis_points: 4_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
            }),
            OperationKind::Smuggling => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 18_000,
                partial_take_basis_points: 4_000,
                recent_take_recovery_window: RECENT_TAKE_RECOVERY_WINDOW,
                immediate_repeat_value_basis_points: IMMEDIATE_REPEAT_TAKE_VALUE_BASIS_POINTS,
            }),
            OperationKind::Burglary
            | OperationKind::Hijacking
            | OperationKind::Surveillance
            | OperationKind::WitnessPressure
            | OperationKind::DocumentTheft
            | OperationKind::Extraction
            | OperationKind::Sabotage
            | OperationKind::Arson => None,
        },
    }
}

fn operation_exposure_evidence_kind(kind: OperationKind) -> EvidenceKind {
    match kind {
        OperationKind::Burglary | OperationKind::DocumentTheft => EvidenceKind::Fingerprint,
        OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Extraction => EvidenceKind::VehicleDescription,
        OperationKind::Intimidation | OperationKind::WitnessPressure => {
            EvidenceKind::WitnessTestimony
        }
        OperationKind::Surveillance => EvidenceKind::Surveillance,
        OperationKind::GamblingEvent => EvidenceKind::FinancialRecord,
        // Sabotage and arson leave physical traces at the scene like any other hands-on crime.
        // Intake evidence cannot be ForensicAnalysis: the legal model derives that kind only
        // from investigator lab work on an already-open case, and a ForensicAnalysis intake
        // draft would be rejected by the evidence-intake gate.
        OperationKind::Sabotage | OperationKind::Arson => EvidenceKind::Fingerprint,
    }
}

fn relevant_operation_intelligence(kind: OperationKind) -> BTreeSet<InformationTopic> {
    let topics: &[InformationTopic] = match kind {
        OperationKind::Burglary => &[
            InformationTopic::TargetSecurity,
            InformationTopic::MarketAccess,
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::DocumentTheft => &[
            InformationTopic::TargetSecurity,
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::Robbery => &[
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::Hijacking | OperationKind::Smuggling | OperationKind::Extraction => &[
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
            InformationTopic::Personnel,
        ],
        OperationKind::Intimidation | OperationKind::WitnessPressure => &[
            InformationTopic::Personnel,
            InformationTopic::Relationship,
            InformationTopic::PoliceActivity,
        ],
        OperationKind::Surveillance => &[
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::Route,
            InformationTopic::PoliceActivity,
        ],
        OperationKind::GamblingEvent => &[
            InformationTopic::PoliceActivity,
            InformationTopic::Personnel,
            InformationTopic::MarketAccess,
        ],
        OperationKind::Sabotage | OperationKind::Arson => &[
            InformationTopic::TargetSecurity,
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
        ],
    };
    topics.iter().copied().collect()
}

fn capability_for_operation_role(role: RoleKind) -> CapabilityKind {
    match role {
        RoleKind::Driver => CapabilityKind::Driving,
        RoleKind::Lookout => CapabilityKind::Surveillance,
        RoleKind::EntrySpecialist => CapabilityKind::Burglary,
        RoleKind::SafeSpecialist => CapabilityKind::Burglary,
        RoleKind::Muscle => CapabilityKind::Violence,
        RoleKind::InsideContact => CapabilityKind::SocialAccess,
        RoleKind::Coordinator => CapabilityKind::Management,
        RoleKind::Surveillance => CapabilityKind::Surveillance,
        RoleKind::Negotiator => CapabilityKind::Negotiation,
    }
}
