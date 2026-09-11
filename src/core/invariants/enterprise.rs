//! Release-safe structural and registry validation for criminal enterprises.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::{MandateRecord, MandateStatus};
use crate::enterprises::{
    EnterpriseCycleRecord, EnterpriseLocation, EnterpriseRecord, EnterpriseStatus,
};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::{Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength};
use crate::registry::Registry;
use crate::world::{BusinessOwner, BusinessRecord, CharacterRecord, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct EnterpriseAuthorityRefs<'a> {
    mandate: &'a MandateRecord,
    manager: &'a CharacterRecord,
    neighborhood: crate::core::id::NeighborhoodId,
    supporting_businesses: Vec<&'a BusinessRecord>,
}

pub(super) fn validate_enterprises(state: &AppState) -> Result<(), StateValidationError> {
    let mut occupied = BTreeMap::new();
    for enterprise in state.enterprises.enterprises() {
        validate_enterprise_record(state, enterprise)?;
        if enterprise.status() != EnterpriseStatus::Retired
            && let Some(existing) =
                occupied.insert((enterprise.kind(), enterprise.location()), enterprise.id())
        {
            return Err(StateValidationError::DuplicateEnterpriseLocation {
                enterprise: enterprise.id(),
                existing,
            });
        }
    }
    let vice_incident_times = derive_vice_incident_times(state)?;
    let mut previous_cycle_at = BTreeMap::new();
    let mut used_transactions = BTreeSet::new();
    for cycle in state.enterprises.cycles() {
        validate_enterprise_cycle(
            state,
            cycle,
            &vice_incident_times,
            &mut previous_cycle_at,
            &mut used_transactions,
        )?;
    }
    Ok(())
}

fn validate_enterprise_record(
    state: &AppState,
    enterprise: &EnterpriseRecord,
) -> Result<(), StateValidationError> {
    if enterprise.version() == 0 {
        return Err(StateValidationError::InvalidEnterpriseRuntime {
            enterprise: enterprise.id(),
        });
    }
    let refs = resolve_enterprise_authority(state, enterprise)?;
    validate_enterprise_accounts(state, enterprise)?;
    validate_enterprise_schedule(state, enterprise)?;
    validate_enterprise_status(state, enterprise, &refs)
}

fn resolve_enterprise_authority<'a>(
    state: &'a AppState,
    enterprise: &EnterpriseRecord,
) -> Result<EnterpriseAuthorityRefs<'a>, StateValidationError> {
    let organization = state
        .world
        .get_organization(enterprise.organization())
        .ok_or(StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        })?;
    if organization.kind() != OrganizationKind::Criminal {
        return Err(StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        });
    }
    let authority = enterprise.authority();
    let mandate = state.delegation.get_mandate(authority.mandate).ok_or(
        StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        },
    )?;
    let manager = state.world.get_character(authority.manager).ok_or(
        StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        },
    )?;
    if mandate.organization() != enterprise.organization()
        || mandate.manager() != authority.manager
        || enterprise.manager() != authority.manager
    {
        return Err(StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        });
    }
    let neighborhood = resolve_enterprise_neighborhood(state, enterprise)?;
    let supporting_businesses = resolve_supporting_businesses(state, enterprise)?;
    Ok(EnterpriseAuthorityRefs {
        mandate,
        manager,
        neighborhood,
        supporting_businesses,
    })
}

fn resolve_enterprise_neighborhood(
    state: &AppState,
    enterprise: &EnterpriseRecord,
) -> Result<crate::core::id::NeighborhoodId, StateValidationError> {
    match enterprise.location() {
        EnterpriseLocation::Neighborhood(id) => {
            state.world.get_neighborhood(id).ok_or(
                StateValidationError::InvalidEnterpriseLocation {
                    enterprise: enterprise.id(),
                },
            )?;
            Ok(id)
        }
        EnterpriseLocation::Business(id) => {
            let business = state.world.get_business(id).ok_or(
                StateValidationError::InvalidEnterpriseLocation {
                    enterprise: enterprise.id(),
                },
            )?;
            state
                .world
                .get_neighborhood(business.neighborhood())
                .ok_or(StateValidationError::InvalidEnterpriseLocation {
                    enterprise: enterprise.id(),
                })?;
            Ok(business.neighborhood())
        }
    }
}

fn resolve_supporting_businesses<'a>(
    state: &'a AppState,
    enterprise: &EnterpriseRecord,
) -> Result<Vec<&'a BusinessRecord>, StateValidationError> {
    let mut businesses = Vec::with_capacity(enterprise.supporting_businesses().len());
    for business_id in enterprise.supporting_businesses() {
        if matches!(enterprise.location(), EnterpriseLocation::Business(location_id) if location_id == *business_id)
        {
            return Err(StateValidationError::InvalidEnterpriseSupportingBusiness {
                enterprise: enterprise.id(),
                business: *business_id,
            });
        }
        businesses.push(state.world.get_business(*business_id).ok_or(
            StateValidationError::InvalidEnterpriseSupportingBusiness {
                enterprise: enterprise.id(),
                business: *business_id,
            },
        )?);
    }
    Ok(businesses)
}

fn validate_enterprise_accounts(
    state: &AppState,
    enterprise: &EnterpriseRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseAccounts {
        enterprise: enterprise.id(),
    };
    let cash = state
        .finance
        .get_account(enterprise.cash_account())
        .ok_or_else(invalid)?;
    let settlement = state
        .finance
        .get_account(enterprise.settlement_account())
        .ok_or_else(invalid)?;
    let expected_owner = FinancialOwner::Organization(enterprise.organization());
    if cash.owner() != expected_owner
        || settlement.owner() != expected_owner
        || !matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        )
        || settlement.kind() != AccountKind::Settlement
        || enterprise.cash_account() == enterprise.settlement_account()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_enterprise_schedule(
    state: &AppState,
    enterprise: &EnterpriseRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseSchedule {
        enterprise: enterprise.id(),
    };
    if enterprise.established_at() > state.now()
        || enterprise
            .last_cycle_at()
            .is_some_and(|last_cycle| last_cycle > state.now())
        || enterprise
            .loss_streak_anchor()
            .is_some_and(|anchor| anchor < enterprise.established_at() || anchor > state.now())
        || enterprise.retired_at().is_some_and(|retired_at| {
            retired_at < enterprise.established_at()
                || retired_at > state.now()
                || enterprise
                    .last_cycle_at()
                    .is_some_and(|last_cycle| last_cycle > retired_at)
                || enterprise
                    .loss_streak_anchor()
                    .is_some_and(|anchor| anchor > retired_at)
        })
        || state
            .enterprises
            .latest_cycle(enterprise.id())
            .map(|cycle| cycle.occurred_at())
            != enterprise.last_cycle_at()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_enterprise_status(
    state: &AppState,
    enterprise: &EnterpriseRecord,
    refs: &EnterpriseAuthorityRefs<'_>,
) -> Result<(), StateValidationError> {
    match enterprise.status() {
        EnterpriseStatus::Active => {
            if enterprise.retired_at().is_some() {
                return Err(StateValidationError::InvalidEnterpriseRuntime {
                    enterprise: enterprise.id(),
                });
            }
            validate_active_enterprise(state, enterprise, refs)
        }
        EnterpriseStatus::Suspended => {
            if enterprise.retired_at().is_some() {
                return Err(StateValidationError::InvalidEnterpriseRuntime {
                    enterprise: enterprise.id(),
                });
            }
            if enterprise.next_cycle_at().is_some() {
                return Err(StateValidationError::InvalidEnterpriseSchedule {
                    enterprise: enterprise.id(),
                });
            }
            Ok(())
        }
        EnterpriseStatus::Retired => {
            if enterprise.retired_at().is_none() {
                return Err(StateValidationError::InvalidEnterpriseRuntime {
                    enterprise: enterprise.id(),
                });
            }
            if enterprise.next_cycle_at().is_some() {
                return Err(StateValidationError::InvalidEnterpriseSchedule {
                    enterprise: enterprise.id(),
                });
            }
            Ok(())
        }
    }
}

fn validate_active_enterprise(
    state: &AppState,
    enterprise: &EnterpriseRecord,
    refs: &EnterpriseAuthorityRefs<'_>,
) -> Result<(), StateValidationError> {
    let authority = enterprise.authority();
    let authority_covers_location =
        crate::enterprises::enterprise_execution::can_authority_cover_location(
            authority.scope,
            enterprise.location(),
            refs.neighborhood,
        );
    let authority_covers_support = refs.supporting_businesses.iter().all(|business| {
        crate::enterprises::enterprise_execution::can_authority_cover_location(
            authority.scope,
            EnterpriseLocation::Business(business.id()),
            business.neighborhood(),
        )
    });
    let host_is_owned = match enterprise.location() {
        EnterpriseLocation::Neighborhood(_) => true,
        EnterpriseLocation::Business(business_id) => state
            .world
            .get_business(business_id)
            .is_some_and(|business| {
                business.owner() == BusinessOwner::Organization(enterprise.organization())
            }),
    };
    if refs.manager.organization() != Some(enterprise.organization())
        || refs.mandate.status() != MandateStatus::Active
        || !refs.mandate.scopes().contains(&authority.scope)
        || !authority_covers_location
        || !authority_covers_support
        || !host_is_owned
        || refs.supporting_businesses.iter().any(|business| {
            business.owner() != BusinessOwner::Organization(enterprise.organization())
        })
    {
        return Err(StateValidationError::InvalidEnterpriseAuthority {
            enterprise: enterprise.id(),
        });
    }
    match enterprise.next_cycle_at() {
        Some(next_cycle_at) => {
            if next_cycle_at <= enterprise.established_at()
                || enterprise
                    .last_cycle_at()
                    .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
            {
                return Err(StateValidationError::InvalidEnterpriseSchedule {
                    enterprise: enterprise.id(),
                });
            }
        }
        None if enterprise.last_cycle_at().is_some() => {}
        None => {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }
    }
    Ok(())
}

fn derive_vice_incident_times(
    state: &AppState,
) -> Result<
    BTreeSet<(crate::core::id::EnterpriseId, crate::core::time::SimTime)>,
    StateValidationError,
> {
    let mut times = BTreeSet::new();
    for investigation in state.legal.investigations() {
        let Some(EntityRef::Enterprise(enterprise_id)) = investigation.origin() else {
            continue;
        };
        if !investigation
            .subjects()
            .contains(&EntityRef::Enterprise(enterprise_id))
        {
            continue;
        }
        let invalid = || StateValidationError::InvalidInvestigationActivity {
            investigation: investigation.id(),
        };
        state
            .enterprises
            .get_enterprise(enterprise_id)
            .ok_or_else(invalid)?;
        let owner = state
            .world
            .get_organization(investigation.owner())
            .ok_or_else(invalid)?;
        if owner.kind() != OrganizationKind::LawEnforcement {
            continue;
        }
        for evidence_id in investigation.evidence() {
            let evidence = state.legal.get_evidence(*evidence_id).ok_or_else(invalid)?;
            if evidence.investigation() == investigation.id()
                && evidence.custodian() == investigation.owner()
                && evidence.subject() == EntityRef::Enterprise(enterprise_id)
                && evidence.origin() == Some(EntityRef::Enterprise(enterprise_id))
                && evidence.source().is_none()
                && evidence.kind() == EvidenceKind::Surveillance
                && evidence.strength() == EvidenceStrength::Weak
                && evidence.reliability() == EvidenceReliability::Questionable
                && evidence.admissibility() == Admissibility::Unknown
            {
                times.insert((enterprise_id, evidence.discovered_at()));
            }
        }
    }
    Ok(times)
}

fn validate_enterprise_cycle(
    state: &AppState,
    cycle: &EnterpriseCycleRecord,
    vice_incident_times: &BTreeSet<(crate::core::id::EnterpriseId, crate::core::time::SimTime)>,
    previous_cycle_at: &mut BTreeMap<crate::core::id::EnterpriseId, crate::core::time::SimTime>,
    used_transactions: &mut BTreeSet<crate::core::id::LedgerTransactionId>,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() };
    let enterprise = state
        .enterprises
        .get_enterprise(cycle.enterprise())
        .ok_or_else(invalid)?;
    let prior = previous_cycle_at.insert(cycle.enterprise(), cycle.occurred_at());
    if cycle.occurred_at() <= enterprise.established_at()
        || cycle.occurred_at() > state.now()
        || prior.is_some_and(|prior| cycle.occurred_at() <= prior)
        || cycle.gross_revenue().cents() < 0
        || cycle.operating_cost().cents() < 0
        || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
    {
        return Err(invalid());
    }
    validate_cycle_attention(state, enterprise, cycle)?;
    validate_cycle_vice(enterprise, cycle, vice_incident_times)?;
    validate_cycle_transaction(state, enterprise, cycle, used_transactions)
}

fn validate_cycle_attention(
    state: &AppState,
    enterprise: &EnterpriseRecord,
    cycle: &EnterpriseCycleRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() };
    match cycle.attention() {
        AttentionClass::Routine => {
            if cycle.information().is_some() {
                return Err(invalid());
            }
        }
        AttentionClass::Notable => {
            let information_id = cycle.information().ok_or_else(invalid)?;
            let information = state
                .intelligence
                .get_information(information_id)
                .ok_or_else(invalid)?;
            if information.holder() != KnowledgeHolder::Organization(enterprise.organization())
                || information.source_kind() != InformationSourceKind::AfterAction
                || information.topic() != InformationTopic::FinancialPerformance
                || information.source_entity() != Some(EntityRef::Character(enterprise.manager()))
                || information.subject() != EntityRef::Enterprise(enterprise.id())
                || information.observed_at() != cycle.occurred_at()
                || information.recorded_at() != cycle.occurred_at()
                || information.reliability() != Reliability::DirectAccess
                || information.specificity() != Specificity::Precise
            {
                return Err(invalid());
            }
        }
        AttentionClass::Exception | AttentionClass::Crisis => return Err(invalid()),
    }
    Ok(())
}

fn validate_cycle_vice(
    enterprise: &EnterpriseRecord,
    cycle: &EnterpriseCycleRecord,
    vice_incident_times: &BTreeSet<(crate::core::id::EnterpriseId, crate::core::time::SimTime)>,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() };
    if cycle.drew_vice_attention()
        != vice_incident_times.contains(&(enterprise.id(), cycle.occurred_at()))
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_cycle_transaction(
    state: &AppState,
    enterprise: &EnterpriseRecord,
    cycle: &EnterpriseCycleRecord,
    used_transactions: &mut BTreeSet<crate::core::id::LedgerTransactionId>,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() };
    match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
        (true, None) => Ok(()),
        (false, Some(transaction_id)) => {
            if !used_transactions.insert(transaction_id) {
                return Err(invalid());
            }
            let transaction = state
                .finance
                .get_transaction(transaction_id)
                .ok_or_else(invalid)?;
            let settlement_cents = cycle.net_cash().cents().checked_neg().ok_or_else(invalid)?;
            let has_cash = transaction.postings().iter().any(|posting| {
                posting.account == enterprise.cash_account() && posting.amount == cycle.net_cash()
            });
            let has_settlement = transaction.postings().iter().any(|posting| {
                posting.account == enterprise.settlement_account()
                    && posting.amount == Money::from_cents(settlement_cents)
            });
            if transaction.occurred_at() != cycle.occurred_at()
                || transaction.postings().len() != 2
                || !has_cash
                || !has_settlement
            {
                return Err(invalid());
            }
            Ok(())
        }
        (true, Some(_)) | (false, None) => Err(invalid()),
    }
}

pub(super) fn validate_enterprises_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for enterprise in state.enterprises.enterprises() {
        validate_enterprise_definition(registry, state, enterprise)?;
    }
    for cycle in state.enterprises.cycles() {
        validate_cycle_against_registry(registry, state, cycle)?;
    }
    Ok(())
}

fn validate_enterprise_definition(
    registry: &Registry,
    state: &AppState,
    enterprise: &EnterpriseRecord,
) -> Result<(), StateValidationError> {
    let definition = registry.get_enterprise(enterprise.kind());
    if enterprise.status() == EnterpriseStatus::Active {
        let schedule_base = [
            Some(enterprise.established_at()),
            enterprise.last_cycle_at(),
            enterprise.loss_streak_anchor(),
        ]
        .into_iter()
        .flatten()
        .max()
        .expect("established enterprise always has a schedule base");
        let expected_next = schedule_base.checked_add(definition.economics().cycle());
        if enterprise.next_cycle_at() != expected_next {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }
    }
    let mut network_functions = BTreeSet::new();
    if let EnterpriseLocation::Business(business_id) = enterprise.location() {
        let business = state.world.get_business(business_id).ok_or(
            StateValidationError::InvalidEnterpriseLocation {
                enterprise: enterprise.id(),
            },
        )?;
        for function in definition.required_business_functions() {
            if !business.has_function(*function) {
                return Err(StateValidationError::EnterpriseBusinessRequirementMissing {
                    enterprise: enterprise.id(),
                    business: business_id,
                    function: *function,
                });
            }
        }
        network_functions.extend(business.functions().iter().copied());
    } else if !definition.required_business_functions().is_empty() {
        return Err(StateValidationError::InvalidEnterpriseLocation {
            enterprise: enterprise.id(),
        });
    }
    for business_id in enterprise.supporting_businesses() {
        let business = state.world.get_business(*business_id).ok_or(
            StateValidationError::InvalidEnterpriseSupportingBusiness {
                enterprise: enterprise.id(),
                business: *business_id,
            },
        )?;
        network_functions.extend(business.functions().iter().copied());
    }
    for function in definition.required_network_functions() {
        if !network_functions.contains(function) {
            return Err(StateValidationError::EnterpriseNetworkRequirementMissing {
                enterprise: enterprise.id(),
                function: *function,
            });
        }
    }
    Ok(())
}

fn validate_cycle_against_registry(
    registry: &Registry,
    state: &AppState,
    cycle: &EnterpriseCycleRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() };
    let enterprise = state
        .enterprises
        .get_enterprise(cycle.enterprise())
        .ok_or_else(invalid)?;
    let economics = registry.get_enterprise(enterprise.kind()).economics();
    let (expected_gross, expected_cost, expected_net) =
        crate::enterprises::enterprise_execution::resolve_historical_enterprise_cycle_financials(
            registry, state, cycle,
        )
        .map_err(|_| invalid())?;
    if cycle.gross_revenue() != expected_gross
        || cycle.operating_cost() != expected_cost
        || cycle.net_cash() != expected_net
    {
        return Err(invalid());
    }
    let variance = i32::from(cycle.variance_basis_points()).unsigned_abs();
    let per_case = economics.heat_surcharge_per_active_case().cents();
    if cycle.investigation_heat().cents() < 0
        || (per_case == 0 && cycle.investigation_heat().cents() != 0)
        || (per_case > 0 && cycle.investigation_heat().cents() % per_case != 0)
    {
        return Err(invalid());
    }
    let previous_heat = state
        .enterprises
        .prior_cycle(cycle.enterprise(), cycle.id())
        .map(|prior| prior.investigation_heat());
    let heat_reportable =
        crate::enterprises::enterprise_execution::enterprise_heat_change_is_reportable(
            previous_heat,
            cycle.investigation_heat(),
        );
    let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
        || heat_reportable
        || cycle.net_cash() < Money::ZERO
        || cycle.drew_vice_attention()
    {
        AttentionClass::Notable
    } else {
        AttentionClass::Routine
    };
    if variance > u32::from(economics.gross_variance_basis_points())
        || cycle.attention() != expected_attention
    {
        return Err(invalid());
    }
    Ok(())
}
