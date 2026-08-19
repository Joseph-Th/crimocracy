//! Validation, lifecycle transitions, and deterministic expiry for provenance-backed operation opportunities.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    IdExhaustionError, IdKind, InformationId, OperationId, OpportunityId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::KnowledgeHolder;
use crate::operations::{OperationKind, OperationObjective, OperationStatus};
use crate::opportunities::{
    OperationOpportunityContext, OperationOpportunityDraft, OpportunityContext, OpportunityRecord,
    OpportunityStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{Lifecycle, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OpportunityError {
    #[error("opportunity summary must not be empty")]
    EmptySummary,
    #[error("opportunity organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("opportunity organization {0} is not active")]
    InactiveOrganization(OrganizationId),
    #[error("opportunity organization {0} is not a criminal organization")]
    InvalidOrganizationKind(OrganizationId),
    #[error("operation opportunity must reference at least one target entity")]
    MissingTargets,
    #[error("opportunity target entity {0:?} does not exist")]
    MissingTarget(EntityRef),
    #[error("operation opportunity must have at least one source-information record")]
    MissingSourceInformation,
    #[error("opportunity source-information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("information {information} is not held by opportunity organization {organization}")]
    InformationUnavailable {
        information: InformationId,
        organization: OrganizationId,
    },
    #[error("information {information} concerns {subject:?}, which is not an opportunity target")]
    InformationTargetMismatch {
        information: InformationId,
        subject: EntityRef,
    },
    #[error("opportunity target {0:?} has no direct source-information record")]
    UncoveredTarget(EntityRef),
    #[error(
        "opportunity validity deadline {valid_until:?} must be later than discovery time {now:?}"
    )]
    InvalidValidityWindow { now: SimTime, valid_until: SimTime },
    #[error("matching open opportunity {0} already exists")]
    ExistingOpenOpportunity(OpportunityId),
    #[error(
        "opportunity discovery was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleDiscoveryTime { expected: SimTime, found: SimTime },
    #[error("opportunity {0} does not exist")]
    MissingOpportunity(OpportunityId),
    #[error("opportunity {opportunity} is not open; current status is {status:?}")]
    OpportunityNotOpen {
        opportunity: OpportunityId,
        status: OpportunityStatus,
    },
    #[error("opportunity {opportunity} expired at {valid_until:?}")]
    OpportunityExpired {
        opportunity: OpportunityId,
        valid_until: SimTime,
    },
    #[error("opportunity {0} has no validity deadline and cannot expire automatically")]
    MissingValidityDeadline(OpportunityId),
    #[error(
        "operation {operation} is scheduled at or after opportunity validity deadline {valid_until:?}"
    )]
    OperationScheduledAfterWindow {
        operation: OperationId,
        valid_until: SimTime,
    },
    #[error(
        "opportunity {opportunity} does not expire until {valid_until:?}; current time is {now:?}"
    )]
    ExpiryNotDue {
        opportunity: OpportunityId,
        valid_until: SimTime,
        now: SimTime,
    },
    #[error(
        "opportunity expiry was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleExpiryTime { expected: SimTime, found: SimTime },
    #[error("opportunity {opportunity} changed after validation; expected version {expected}, found {found}")]
    StaleOpportunity {
        opportunity: OpportunityId,
        expected: u32,
        found: u32,
    },
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {operation} is not authorized for opportunity conversion")]
    OperationNotAuthorized { operation: OperationId },
    #[error("operation {operation} changed after opportunity conversion validation; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error("operation {operation} belongs to organization {operation_organization}, not opportunity organization {opportunity_organization}")]
    OperationOrganizationMismatch {
        operation: OperationId,
        operation_organization: OrganizationId,
        opportunity_organization: OrganizationId,
    },
    #[error("operation {operation} kind {operation_kind:?} does not match opportunity kind {opportunity_kind:?}")]
    OperationKindMismatch {
        operation: OperationId,
        operation_kind: OperationKind,
        opportunity_kind: OperationKind,
    },
    #[error("operation {operation} targets do not exactly match opportunity targets")]
    OperationTargetsMismatch { operation: OperationId },
    #[error("operation {operation} does not use the property-acquisition objective required by its opportunity")]
    OperationObjectiveMismatch { operation: OperationId },
    #[error("operation {operation} is already linked to opportunity {opportunity}")]
    OperationAlreadyLinked {
        operation: OperationId,
        opportunity: OpportunityId,
    },
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedOpportunityDiscovery {
    draft: OperationOpportunityDraft,
    discovered_at: SimTime,
    report: ValidatedReport,
}

impl ValidatedOpportunityDiscovery {
    pub fn commit(self, state: &mut AppState) -> Result<OpportunityId, OpportunityError> {
        state
            .ids
            .reserve_many(&[(IdKind::Report, 1), (IdKind::Opportunity, 1)])?;
        if state.now() != self.discovered_at {
            return Err(OpportunityError::StaleDiscoveryTime {
                expected: self.discovered_at,
                found: state.now(),
            });
        }
        validate_discovery_state(state, &self.draft, self.discovered_at)?;
        let report = self.report.commit(state)?;
        let id = state.ids.next_opportunity()?;
        state.opportunities.insert(OpportunityRecord {
            id,
            organization: self.draft.organization,
            context: OpportunityContext::Operation(OperationOpportunityContext {
                operation_kind: self.draft.operation_kind,
                targets: self.draft.targets,
            }),
            discovered_at: self.discovered_at,
            valid_until: self.draft.valid_until,
            source_information: self.draft.source_information,
            summary: self.draft.summary,
            report,
            resolution: None,
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_discover_operation_opportunity(
    registry: &Registry,
    state: &AppState,
    draft: OperationOpportunityDraft,
) -> Result<ValidatedOpportunityDiscovery, OpportunityError> {
    let discovered_at = state.now();
    let definition = registry.get_operation(draft.operation_kind);
    validate_discovery_state(state, &draft, discovered_at)?;

    let mut entities = draft.targets.clone();
    entities.insert(EntityRef::Organization(draft.organization));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: draft.organization,
            kind: ReportKind::Opportunity,
            title: format!("{} opportunity", definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: draft.summary.clone(),
                sources: draft.source_information.iter().copied().collect(),
                entities,
                decision: None,
            }],
        },
    )?;

    Ok(ValidatedOpportunityDiscovery {
        draft,
        discovered_at,
        report,
    })
}

fn validate_discovery_state(
    state: &AppState,
    draft: &OperationOpportunityDraft,
    discovered_at: SimTime,
) -> Result<(), OpportunityError> {
    if draft.summary.trim().is_empty() {
        return Err(OpportunityError::EmptySummary);
    }
    let organization = state
        .world
        .get_organization(draft.organization)
        .ok_or(OpportunityError::MissingOrganization(draft.organization))?;
    if organization.lifecycle() != Lifecycle::Active {
        return Err(OpportunityError::InactiveOrganization(draft.organization));
    }
    if organization.kind() != OrganizationKind::Criminal {
        return Err(OpportunityError::InvalidOrganizationKind(
            draft.organization,
        ));
    }
    if draft.targets.is_empty() {
        return Err(OpportunityError::MissingTargets);
    }
    for target in &draft.targets {
        if !is_entity_present(state, *target) {
            return Err(OpportunityError::MissingTarget(*target));
        }
    }
    if draft.source_information.is_empty() {
        return Err(OpportunityError::MissingSourceInformation);
    }
    let mut covered_targets = BTreeSet::new();
    for source in &draft.source_information {
        let information = state
            .intelligence
            .get_information(*source)
            .ok_or(OpportunityError::MissingInformation(*source))?;
        if information.holder() != KnowledgeHolder::Organization(draft.organization) {
            return Err(OpportunityError::InformationUnavailable {
                information: *source,
                organization: draft.organization,
            });
        }
        if !draft.targets.contains(&information.subject()) {
            return Err(OpportunityError::InformationTargetMismatch {
                information: *source,
                subject: information.subject(),
            });
        }
        covered_targets.insert(information.subject());
    }
    if let Some(uncovered) = draft
        .targets
        .iter()
        .find(|target| !covered_targets.contains(target))
    {
        return Err(OpportunityError::UncoveredTarget(*uncovered));
    }
    if let Some(valid_until) = draft.valid_until {
        if valid_until <= discovered_at {
            return Err(OpportunityError::InvalidValidityWindow {
                now: discovered_at,
                valid_until,
            });
        }
    }
    if let Some(existing) = state.opportunities.open_matching_operation(
        draft.organization,
        draft.operation_kind,
        &draft.targets,
    ) {
        return Err(OpportunityError::ExistingOpenOpportunity(existing.id()));
    }
    Ok(())
}

pub struct ValidatedOpportunityDismissal {
    opportunity: OpportunityId,
    expected_version: u32,
}

impl ValidatedOpportunityDismissal {
    pub fn commit(self, state: &mut AppState) -> Result<(), OpportunityError> {
        let record = validate_open_opportunity(state, self.opportunity)?;
        if record.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_not_expired(state, record)?;
        state.opportunities.dismiss(self.opportunity, state.now());
        Ok(())
    }
}

pub fn validate_dismiss_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<ValidatedOpportunityDismissal, OpportunityError> {
    let record = validate_open_opportunity(state, opportunity)?;
    validate_not_expired(state, record)?;
    Ok(ValidatedOpportunityDismissal {
        opportunity,
        expected_version: record.version(),
    })
}

pub struct ValidatedOpportunityConversion {
    opportunity: OpportunityId,
    expected_opportunity_version: u32,
    operation: OperationId,
    expected_operation_version: u32,
}

impl ValidatedOpportunityConversion {
    pub fn commit(self, state: &mut AppState) -> Result<(), OpportunityError> {
        let opportunity = validate_open_opportunity(state, self.opportunity)?;
        if opportunity.version() != self.expected_opportunity_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_opportunity_version,
                found: opportunity.version(),
            });
        }
        validate_not_expired(state, opportunity)?;
        let operation = state
            .operations
            .get_operation(self.operation)
            .ok_or(OpportunityError::MissingOperation(self.operation))?;
        if operation.version() != self.expected_operation_version {
            return Err(OpportunityError::StaleOperation {
                operation: self.operation,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        validate_conversion_match(state, opportunity, operation)?;
        state
            .opportunities
            .convert(self.opportunity, self.operation, state.now());
        Ok(())
    }
}

pub fn validate_convert_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
    operation: OperationId,
) -> Result<ValidatedOpportunityConversion, OpportunityError> {
    let opportunity_record = validate_open_opportunity(state, opportunity)?;
    validate_not_expired(state, opportunity_record)?;
    let operation_record = state
        .operations
        .get_operation(operation)
        .ok_or(OpportunityError::MissingOperation(operation))?;
    validate_conversion_match(state, opportunity_record, operation_record)?;
    Ok(ValidatedOpportunityConversion {
        opportunity,
        expected_opportunity_version: opportunity_record.version(),
        operation,
        expected_operation_version: operation_record.version(),
    })
}

fn validate_open_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<&OpportunityRecord, OpportunityError> {
    let record = state
        .opportunities
        .get_opportunity(opportunity)
        .ok_or(OpportunityError::MissingOpportunity(opportunity))?;
    if record.status() != OpportunityStatus::Open {
        return Err(OpportunityError::OpportunityNotOpen {
            opportunity,
            status: record.status(),
        });
    }
    Ok(record)
}

fn validate_not_expired(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), OpportunityError> {
    if let Some(valid_until) = opportunity.valid_until() {
        if state.now() >= valid_until {
            return Err(OpportunityError::OpportunityExpired {
                opportunity: opportunity.id(),
                valid_until,
            });
        }
    }
    Ok(())
}

fn validate_conversion_match(
    state: &AppState,
    opportunity: &OpportunityRecord,
    operation: &crate::operations::OperationRecord,
) -> Result<(), OpportunityError> {
    if operation.status() != OperationStatus::Authorized {
        return Err(OpportunityError::OperationNotAuthorized {
            operation: operation.id(),
        });
    }
    if let Some(existing) = state
        .opportunities
        .opportunity_for_operation(operation.id())
    {
        return Err(OpportunityError::OperationAlreadyLinked {
            operation: operation.id(),
            opportunity: existing.id(),
        });
    }
    if operation.responsible_organization() != opportunity.organization() {
        return Err(OpportunityError::OperationOrganizationMismatch {
            operation: operation.id(),
            operation_organization: operation.responsible_organization(),
            opportunity_organization: opportunity.organization(),
        });
    }
    let context = opportunity.context().operation();
    if operation.kind() != context.operation_kind() {
        return Err(OpportunityError::OperationKindMismatch {
            operation: operation.id(),
            operation_kind: operation.kind(),
            opportunity_kind: context.operation_kind(),
        });
    }
    if context.operation_kind().supports_property_acquisition()
        && !matches!(
            operation.objective(),
            OperationObjective::AcquireProperty { .. }
        )
    {
        return Err(OpportunityError::OperationObjectiveMismatch {
            operation: operation.id(),
        });
    }
    let operation_targets: BTreeSet<_> = operation
        .objective()
        .referenced_entities()
        .into_iter()
        .collect();
    if operation_targets != *context.targets() {
        return Err(OpportunityError::OperationTargetsMismatch {
            operation: operation.id(),
        });
    }
    // The opportunity window is meaningful: converting must bind an operation that will actually
    // execute inside the window, not one scheduled after the opportunity has closed. Otherwise the
    // "valid until" deadline could be consumed by an operation that never runs while the situation
    // was live.
    if opportunity
        .valid_until()
        .is_some_and(|valid_until| operation.scheduled_for() >= valid_until)
    {
        return Err(OpportunityError::OperationScheduledAfterWindow {
            operation: operation.id(),
            valid_until: opportunity
                .valid_until()
                .expect("expiry-checked opportunity has a validity window"),
        });
    }
    Ok(())
}

struct ValidatedOpportunityExpiry {
    opportunity: OpportunityId,
    expected_version: u32,
    expected_now: SimTime,
    valid_until: SimTime,
    report: ValidatedReport,
}

impl ValidatedOpportunityExpiry {
    fn commit(self, state: &mut AppState) -> Result<ReportId, OpportunityError> {
        if state.now() != self.expected_now {
            return Err(OpportunityError::StaleExpiryTime {
                expected: self.expected_now,
                found: state.now(),
            });
        }
        let opportunity = validate_open_opportunity(state, self.opportunity)?;
        if opportunity.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: opportunity.version(),
            });
        }
        let valid_until = validate_expiry_due(state, opportunity)?;
        debug_assert_eq!(valid_until, self.valid_until);

        let report = self.report.commit(state)?;
        state
            .opportunities
            .expire(self.opportunity, self.valid_until, report);
        Ok(report)
    }
}

fn validate_expire_opportunity(
    registry: &Registry,
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<ValidatedOpportunityExpiry, OpportunityError> {
    let record = validate_open_opportunity(state, opportunity)?;
    let valid_until = validate_expiry_due(state, record)?;
    let definition = registry.get_operation(record.context().operation().operation_kind());
    let mut entities = record.context().operation().targets().clone();
    entities.insert(EntityRef::Organization(record.organization()));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.organization(),
            kind: ReportKind::Opportunity,
            title: format!("{} opportunity expired", definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!("Opportunity expired: {}", record.summary()),
                sources: record.source_information().iter().copied().collect(),
                entities,
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedOpportunityExpiry {
        opportunity,
        expected_version: record.version(),
        expected_now: state.now(),
        valid_until,
        report,
    })
}

fn validate_expiry_due(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<SimTime, OpportunityError> {
    let valid_until = opportunity
        .valid_until()
        .ok_or(OpportunityError::MissingValidityDeadline(opportunity.id()))?;
    if state.now() < valid_until {
        return Err(OpportunityError::ExpiryNotDue {
            opportunity: opportunity.id(),
            valid_until,
            now: state.now(),
        });
    }
    Ok(valid_until)
}

pub(crate) fn expire_due_opportunities(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<OpportunityId> {
    let due = state.opportunities.due_expiring_at_or_before(state.now());
    let mut expired = Vec::with_capacity(due.len());
    for opportunity in due {
        validate_expire_opportunity(registry, state, opportunity)
            .expect("due opportunity must validate an expiry transaction")
            .commit(state)
            .expect("validated opportunity expiry must commit atomically");
        expired.push(opportunity);
    }
    expired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::core::time::SimDuration;
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
        Specificity,
    };
    use crate::operations::operation_system::{
        apply_transition, validate_authorize_operation, OperationTransition,
    };
    use crate::operations::{OperationApproach, OperationDraft, OperationObjective, RoleKind};
    use crate::opportunities::OpportunityResolution;
    use crate::world::world_system::{
        designate_player_organization, insert_business, insert_character, insert_organization,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CapabilityKind, CharacterDraft, OrganizationDraft,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct OpportunityFixture {
        registry: Registry,
        state: AppState,
        organization: OrganizationId,
        business: crate::core::id::BusinessId,
        leader: crate::core::id::CharacterId,
        entry_specialist: crate::core::id::CharacterId,
        source: InformationId,
    }

    fn make_fixture() -> OpportunityFixture {
        let registry = build_registry();
        let mut state = AppState::new(0x0F90_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Opportunity Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal organization fixture should validate");
        let neighborhood = crate::world::world_system::insert_neighborhood(
            &mut state,
            crate::world::NeighborhoodDraft {
                name: "Bellmore Ward".to_owned(),
                profile: crate::world::NeighborhoodProfile {
                    economy: crate::world::NeighborhoodEconomyProfile {
                        wealth: crate::world::Rating::try_new(70).unwrap(),
                        commercial_activity: crate::world::Rating::try_new(70).unwrap(),
                        illicit_demand: crate::world::Rating::try_new(40).unwrap(),
                    },
                    institutions: crate::world::NeighborhoodInstitutionProfile {
                        police_presence: crate::world::Rating::try_new(45).unwrap(),
                        political_influence: crate::world::Rating::try_new(50).unwrap(),
                        social_cohesion: crate::world::Rating::try_new(55).unwrap(),
                        visible_violence_tolerance: crate::world::Rating::try_new(20).unwrap(),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Bellmore Jewelry".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("business fixture should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Opportunity Crew Leader".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("leader fixture should validate");
        let entry_specialist = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Opportunity Entry Specialist".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Burglary,
                    crate::world::Rating::try_new(70).unwrap(),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("entry specialist fixture should validate");
        let source = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind: InformationSourceKind::Informant,
                topic: InformationTopic::TargetSecurity,
                source_entity: None,
                subject: EntityRef::Business(business),
                observed_at: state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::General,
                summary: "A jewelry delivery is expected on Thursday and the night security appears light."
                    .to_owned(),
            },
        )
        .expect("opportunity source information should validate")
        .commit(&mut state)
        .expect("opportunity source information should commit");
        OpportunityFixture {
            registry,
            state,
            organization,
            business,
            leader,
            entry_specialist,
            source,
        }
    }

    fn opportunity_draft(
        fixture: &OpportunityFixture,
        valid_until: SimTime,
    ) -> OperationOpportunityDraft {
        OperationOpportunityDraft {
            organization: fixture.organization,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(fixture.business)]),
            source_information: BTreeSet::from([fixture.source]),
            summary: "Bellmore Jewelry may be vulnerable around its Thursday delivery window."
                .to_owned(),
            valid_until: Some(valid_until),
        }
    }

    fn authorize_operation(
        fixture: &mut OpportunityFixture,
        objective: OperationObjective,
    ) -> OperationId {
        validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Bellmore Jewelry burglary".to_owned(),
                kind: OperationKind::Burglary,
                responsible_organization: fixture.organization,
                leader: fixture.leader,
                objective,
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([
                    (RoleKind::Coordinator, fixture.leader),
                    (RoleKind::EntrySpecialist, fixture.entry_specialist),
                ]),
                intelligence: BTreeSet::from([fixture.source]),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::from_minutes(10),
            },
        )
        .expect("matching opportunity operation should validate")
        .commit(&mut fixture.state)
        .expect("matching opportunity operation should commit")
    }

    fn authorize_matching_operation(fixture: &mut OpportunityFixture) -> OperationId {
        authorize_operation(
            fixture,
            OperationObjective::AcquireProperty {
                target: EntityRef::Business(fixture.business),
            },
        )
    }

    #[test]
    fn discovery_requires_organization_knowledge_and_creates_a_provenance_report() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("organization-held target information should support an opportunity")
        .commit(&mut fixture.state)
        .expect("validated opportunity discovery should commit");

        let record = fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("opportunity should persist");
        assert_eq!(record.status(), OpportunityStatus::Open);
        assert_eq!(
            record.source_information(),
            &BTreeSet::from([fixture.source])
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunities_for_entity(EntityRef::Business(fixture.business))
                .map(OpportunityRecord::id)
                .collect::<Vec<_>>(),
            vec![opportunity]
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunities_from_information(fixture.source)
                .map(OpportunityRecord::id)
                .collect::<Vec<_>>(),
            vec![opportunity]
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunity_for_report(record.report())
                .map(OpportunityRecord::id),
            Some(opportunity)
        );
        let report = fixture
            .state
            .reports()
            .get_report(record.report())
            .expect("opportunity discovery report should persist");
        assert_eq!(report.kind(), ReportKind::Opportunity);
        assert_eq!(report.recipient(), fixture.organization);
        assert_eq!(report.entries().len(), 1);
        assert_eq!(report.entries()[0].sources, vec![fixture.source]);
        assert!(report.entries()[0]
            .entities
            .contains(&EntityRef::Business(fixture.business)));
        validate_state(&fixture.state).expect("discovered opportunity state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn duplicate_open_opportunity_is_rejected_but_dismissal_allows_later_rediscovery() {
        let mut fixture = make_fixture();
        let draft = opportunity_draft(&fixture, SimTime::from_minutes(120));
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            draft.clone(),
        )
        .expect("first opportunity should validate")
        .commit(&mut fixture.state)
        .expect("first opportunity should commit");
        assert_eq!(
            validate_discover_operation_opportunity(
                &fixture.registry,
                &fixture.state,
                draft.clone()
            )
            .err()
            .expect("duplicate open opportunity should fail"),
            OpportunityError::ExistingOpenOpportunity(opportunity)
        );
        validate_dismiss_opportunity(&fixture.state, opportunity)
            .expect("open opportunity should be dismissible")
            .commit(&mut fixture.state)
            .expect("dismissal should commit");
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("dismissed opportunity should persist")
                .status(),
            OpportunityStatus::Dismissed
        );
        let replacement =
            validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft)
                .expect("dismissed opportunity should not block a later rediscovery")
                .commit(&mut fixture.state)
                .expect("replacement opportunity should commit");
        assert_ne!(replacement, opportunity);
        validate_invariants(&fixture.state);
    }

    #[test]
    fn conversion_requires_exact_authorized_operation_and_survives_save_round_trip() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("opportunity should validate")
        .commit(&mut fixture.state)
        .expect("opportunity should commit");
        let operation = authorize_matching_operation(&mut fixture);

        validate_convert_opportunity(&fixture.state, opportunity, operation)
            .expect("matching authorized operation should convert the opportunity")
            .commit(&mut fixture.state)
            .expect("validated opportunity conversion should commit");
        let record = fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("converted opportunity should persist");
        assert_eq!(record.status(), OpportunityStatus::Converted);
        assert_eq!(
            record
                .resolution()
                .and_then(OpportunityResolution::operation),
            Some(operation)
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunity_for_operation(operation)
                .map(OpportunityRecord::id),
            Some(opportunity)
        );

        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("converted opportunity should build a valid save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&fixture.registry, decoded)
            .expect("converted opportunity save should restore");
        assert_eq!(
            restored
                .opportunities()
                .opportunity_for_operation(operation)
                .map(OpportunityRecord::id),
            Some(opportunity)
        );
        validate_state(&restored).expect("restored opportunity state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn opportunity_expiry_runs_in_stable_tick_pipeline_and_releases_duplicate_key() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(2)),
        )
        .expect("short-lived opportunity should validate")
        .commit(&mut fixture.state)
        .expect("short-lived opportunity should commit");
        let first = run_tick(&fixture.registry, &mut fixture.state);
        assert!(first.expired_opportunities.is_empty());
        let second = run_tick(&fixture.registry, &mut fixture.state);
        assert_eq!(second.expired_opportunities, vec![opportunity]);
        let record = fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("expired opportunity should remain historical");
        assert_eq!(record.status(), OpportunityStatus::Expired);
        let expiry_report = match record.resolution() {
            Some(OpportunityResolution::Expired { at, report }) => {
                assert_eq!(at, SimTime::from_minutes(2));
                report
            }
            resolution => panic!("expected expired opportunity, found {resolution:?}"),
        };
        let report = fixture
            .state
            .reports()
            .get_report(expiry_report)
            .expect("opportunity expiry report should persist");
        assert_eq!(report.kind(), ReportKind::Opportunity);
        assert_eq!(report.generated_at(), SimTime::from_minutes(2));
        assert_eq!(report.entries().len(), 1);
        assert_eq!(
            report.entries()[0].summary,
            "Opportunity expired: Bellmore Jewelry may be vulnerable around its Thursday delivery window."
        );
        assert_eq!(report.entries()[0].sources, vec![fixture.source]);
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunity_for_report(expiry_report)
                .map(OpportunityRecord::id),
            Some(opportunity)
        );

        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("expired opportunity should build a valid save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&fixture.registry, decoded)
            .expect("expired opportunity save should restore");
        let restored_record = restored
            .opportunities()
            .get_opportunity(opportunity)
            .expect("expired opportunity should survive save/load");
        assert_eq!(restored_record.status(), OpportunityStatus::Expired);
        assert_eq!(
            restored_record
                .resolution()
                .and_then(OpportunityResolution::report),
            Some(expiry_report)
        );
        assert_eq!(
            restored
                .opportunities()
                .opportunity_for_report(expiry_report)
                .map(OpportunityRecord::id),
            Some(opportunity)
        );
        validate_state(&restored).expect("restored expired opportunity state should validate");
        validate_invariants(&restored);

        validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("expired opportunity should release its duplicate key")
        .commit(&mut fixture.state)
        .expect("replacement opportunity should commit");
        validate_state(&fixture.state).expect("expired opportunity state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn expiry_report_reaches_later_executive_brief_after_discovery_window_has_closed() {
        let mut fixture = make_fixture();
        designate_player_organization(&mut fixture.state, fixture.organization)
            .expect("criminal fixture organization should be eligible as player organization");
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(2_880)),
        )
        .expect("two-day opportunity should validate")
        .commit(&mut fixture.state)
        .expect("two-day opportunity should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        let first = run_tick(&fixture.registry, &mut fixture.state);
        let first_brief = first
            .executive_brief
            .expect("first daily boundary should create an executive brief");
        assert!(fixture
            .state
            .reports()
            .get_report(first_brief)
            .expect("first executive brief should persist")
            .entries()
            .iter()
            .any(|entry| entry.summary
                == "Bellmore Jewelry may be vulnerable around its Thursday delivery window."));

        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        let second = run_tick(&fixture.registry, &mut fixture.state);
        assert_eq!(second.expired_opportunities, vec![opportunity]);
        let second_brief = second
            .executive_brief
            .expect("second daily boundary should create an executive brief");
        let entries = fixture
            .state
            .reports()
            .get_report(second_brief)
            .expect("second executive brief should persist")
            .entries();
        assert!(entries.iter().any(|entry| {
            entry.summary
                == "Opportunity expired: Bellmore Jewelry may be vulnerable around its Thursday delivery window."
        }));
        assert!(!entries.iter().any(|entry| {
            entry.summary
                == "Bellmore Jewelry may be vulnerable around its Thursday delivery window."
        }));
        validate_state(&fixture.state)
            .expect("expiry-report executive-brief integration should remain valid");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn expiry_token_rejects_clock_staleness_without_partial_report_mutation() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(2)),
        )
        .expect("short-lived opportunity should validate")
        .commit(&mut fixture.state)
        .expect("short-lived opportunity should commit");
        fixture.state.advance_clock(SimDuration::from_minutes(2));
        let expiry = validate_expire_opportunity(&fixture.registry, &fixture.state, opportunity)
            .expect("due opportunity expiry should validate");
        let report_count_before = fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .count();
        fixture.state.advance_clock(SimDuration::ONE_MINUTE);

        assert_eq!(
            expiry
                .commit(&mut fixture.state)
                .expect_err("clock movement must stale a validated expiry transaction"),
            OpportunityError::StaleExpiryTime {
                expected: SimTime::from_minutes(2),
                found: SimTime::from_minutes(3),
            }
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("stale expiry must preserve opportunity")
                .status(),
            OpportunityStatus::Open
        );
        assert_eq!(
            fixture
                .state
                .reports()
                .reports_for(fixture.organization)
                .count(),
            report_count_before
        );

        let expiry_report =
            validate_expire_opportunity(&fixture.registry, &fixture.state, opportunity)
                .expect("overdue opportunity should support a fresh expiry transaction")
                .commit(&mut fixture.state)
                .expect("fresh overdue expiry should commit atomically");
        let record = fixture
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("fresh expiry should preserve historical opportunity");
        assert_eq!(record.status(), OpportunityStatus::Expired);
        assert_eq!(
            record.resolution(),
            Some(OpportunityResolution::Expired {
                at: SimTime::from_minutes(2),
                report: expiry_report,
            })
        );
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(expiry_report)
                .expect("fresh expiry report should persist")
                .generated_at(),
            SimTime::from_minutes(3)
        );
        validate_state(&fixture.state).expect("fresh overdue expiry should restore valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn conversion_token_rejects_operation_lifecycle_change_without_mutating_opportunity() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("opportunity should validate")
        .commit(&mut fixture.state)
        .expect("opportunity should commit");
        let operation = authorize_matching_operation(&mut fixture);
        let conversion = validate_convert_opportunity(&fixture.state, opportunity, operation)
            .expect("fresh conversion should validate");
        fixture.state.advance_clock(SimDuration::from_minutes(10));
        apply_transition(
            &fixture.registry,
            &mut fixture.state,
            operation,
            OperationTransition::Begin,
        )
        .expect("operation should begin after conversion validation");

        let error = conversion
            .commit(&mut fixture.state)
            .expect_err("started operation must stale the older conversion token");
        assert!(matches!(
            error,
            OpportunityError::StaleOperation { operation: id, .. } if id == operation
        ));
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("stale conversion must leave opportunity present")
                .status(),
            OpportunityStatus::Open
        );
        assert!(fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .is_none());
        validate_invariants(&fixture.state);
    }

    #[test]
    fn discovery_rejects_personal_and_foreign_knowledge_without_partial_mutation() {
        let mut fixture = make_fixture();
        let personal_source = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(fixture.leader),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::TargetSecurity,
                source_entity: None,
                subject: EntityRef::Business(fixture.business),
                observed_at: fixture.state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Specific,
                summary: "The crew leader personally observed the rear service entrance."
                    .to_owned(),
            },
        )
        .expect("personal source fixture should validate")
        .commit(&mut fixture.state)
        .expect("personal source fixture should commit");
        let foreign_organization = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Foreign Information Holder".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("foreign organization fixture should validate");
        let foreign_source = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(foreign_organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::TargetSecurity,
                source_entity: None,
                subject: EntityRef::Business(fixture.business),
                observed_at: fixture.state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Specific,
                summary: "Another organization mapped the jewelry store's rear entrance."
                    .to_owned(),
            },
        )
        .expect("foreign source fixture should validate")
        .commit(&mut fixture.state)
        .expect("foreign source fixture should commit");

        for unavailable in [personal_source, foreign_source] {
            let mut draft = opportunity_draft(&fixture, SimTime::from_minutes(120));
            draft.source_information = BTreeSet::from([unavailable]);
            assert_eq!(
                validate_discover_operation_opportunity(&fixture.registry, &fixture.state, draft,)
                    .err()
                    .expect("non-organizational knowledge must not support opportunity discovery"),
                OpportunityError::InformationUnavailable {
                    information: unavailable,
                    organization: fixture.organization,
                }
            );
        }
        assert_eq!(
            fixture
                .state
                .opportunities()
                .opportunities_for_organization(fixture.organization)
                .count(),
            0
        );
        assert_eq!(
            fixture
                .state
                .reports()
                .reports_for(fixture.organization)
                .count(),
            0
        );
        validate_state(&fixture.state)
            .expect("rejected opportunity discovery must leave structurally valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn conversion_rejects_mismatched_operation_kind_without_consuming_opportunity() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("burglary opportunity should validate")
        .commit(&mut fixture.state)
        .expect("burglary opportunity should commit");
        let operation = validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Bellmore Jewelry intimidation".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: fixture.organization,
                leader: fixture.leader,
                objective: OperationObjective::Frighten {
                    target: EntityRef::Business(fixture.business),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, fixture.leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::from_minutes(10),
            },
        )
        .expect("mismatched operation fixture should still be independently valid")
        .commit(&mut fixture.state)
        .expect("mismatched operation fixture should commit");

        assert_eq!(
            validate_convert_opportunity(&fixture.state, opportunity, operation)
                .err()
                .expect("wrong operation kind must not consume opportunity"),
            OpportunityError::OperationKindMismatch {
                operation,
                operation_kind: OperationKind::Intimidation,
                opportunity_kind: OperationKind::Burglary,
            }
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("rejected conversion must preserve opportunity")
                .status(),
            OpportunityStatus::Open
        );
        assert!(fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .is_none());
        validate_state(&fixture.state)
            .expect("mismatched conversion rejection must leave valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn property_opportunity_rejects_same_kind_non_property_operation() {
        let mut fixture = make_fixture();
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(120)),
        )
        .expect("burglary opportunity should validate")
        .commit(&mut fixture.state)
        .expect("burglary opportunity should commit");
        let business = fixture.business;
        let operation = authorize_operation(
            &mut fixture,
            OperationObjective::Frighten {
                target: EntityRef::Business(business),
            },
        );

        assert_eq!(
            validate_convert_opportunity(&fixture.state, opportunity, operation)
                .err()
                .expect("property opportunity must require property objective"),
            OpportunityError::OperationObjectiveMismatch { operation }
        );
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("rejected conversion must preserve opportunity")
                .status(),
            OpportunityStatus::Open
        );
        assert!(fixture
            .state
            .opportunities()
            .opportunity_for_operation(operation)
            .is_none());
        validate_state(&fixture.state)
            .expect("rejected conversion must leave structurally valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn operations_without_property_effects_reject_property_objectives() {
        let fixture = make_fixture();
        let error = validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Invalid intimidation seizure".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: fixture.organization,
                leader: fixture.leader,
                objective: OperationObjective::AcquireProperty {
                    target: EntityRef::Business(fixture.business),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, fixture.leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now(),
            },
        )
        .expect_err("an operation without a property effect must reject property acquisition");

        assert_eq!(
            error,
            crate::operations::operation_system::OperationError::InvalidObjectiveForKind {
                kind: OperationKind::Intimidation,
                objective: crate::operations::OperationObjectiveKind::AcquireProperty,
            }
        );
        assert_eq!(fixture.state.operations().operations().count(), 0);
        validate_state(&fixture.state)
            .expect("rejected property objective must leave structurally valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn opportunity_report_flows_into_next_executive_brief() {
        let mut fixture = make_fixture();
        designate_player_organization(&mut fixture.state, fixture.organization)
            .expect("criminal fixture organization should be eligible as player organization");
        let summary = "Bellmore Jewelry may be vulnerable around its Thursday delivery window.";
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(2_000)),
        )
        .expect("player opportunity should validate")
        .commit(&mut fixture.state)
        .expect("player opportunity should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));

        let tick = run_tick(&fixture.registry, &mut fixture.state);
        assert_eq!(tick.now, SimTime::from_minutes(1_440));
        assert!(tick.expired_opportunities.is_empty());
        let executive_brief = tick
            .executive_brief
            .expect("daily boundary should synthesize the opportunity report");
        let brief = fixture
            .state
            .reports()
            .get_report(executive_brief)
            .expect("executive brief should persist");
        assert_eq!(brief.kind(), ReportKind::ExecutiveBrief);
        assert!(brief.entries().iter().any(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == summary
                && entry
                    .entities
                    .contains(&EntityRef::Business(fixture.business))
        }));
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("long-lived opportunity should remain open")
                .status(),
            OpportunityStatus::Open
        );
        validate_state(&fixture.state)
            .expect("executive-brief opportunity integration should remain valid");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn converted_opportunity_discovery_is_not_resurfaced_in_later_executive_brief() {
        let mut fixture = make_fixture();
        designate_player_organization(&mut fixture.state, fixture.organization)
            .expect("criminal fixture organization should be eligible as player organization");
        let summary = "Bellmore Jewelry may be vulnerable around its Thursday delivery window.";
        let opportunity = validate_discover_operation_opportunity(
            &fixture.registry,
            &fixture.state,
            opportunity_draft(&fixture, SimTime::from_minutes(2_000)),
        )
        .expect("player opportunity should validate")
        .commit(&mut fixture.state)
        .expect("player opportunity should commit");
        let operation = authorize_matching_operation(&mut fixture);
        validate_convert_opportunity(&fixture.state, opportunity, operation)
            .expect("matching operation should convert the opportunity")
            .commit(&mut fixture.state)
            .expect("opportunity conversion should commit");

        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        let tick = run_tick(&fixture.registry, &mut fixture.state);
        let executive_brief = tick
            .executive_brief
            .expect("daily boundary should synthesize an executive brief");
        let brief = fixture
            .state
            .reports()
            .get_report(executive_brief)
            .expect("executive brief should persist");
        assert!(brief.entries().iter().all(|entry| entry.summary != summary));
        assert_eq!(
            fixture
                .state
                .opportunities()
                .get_opportunity(opportunity)
                .expect("converted opportunity should persist")
                .status(),
            OpportunityStatus::Converted
        );
        validate_state(&fixture.state)
            .expect("converted-opportunity brief filtering should remain structurally valid");
        validate_invariants(&fixture.state);
    }
}
