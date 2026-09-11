//! Shared enterprise environment, dependency, reporting, and legal-pressure helpers.

use super::*;

pub(super) fn validate_enterprise_environment(
    state: &AppState,
    organization: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<(), EnterpriseError> {
    let _ = state
        .world
        .get_organization(organization)
        .ok_or(EnterpriseError::InvalidOrganization(organization))?;
    let resolved = resolve_mandate_authority(state, authority)?;
    if resolved.organization() != organization {
        return Err(EnterpriseError::AuthorityOrganizationMismatch {
            authority_organization: resolved.organization(),
            enterprise_organization: organization,
        });
    }
    let neighborhood = resolve_location_neighborhood(state, location)?;
    if !can_authority_cover_location(authority.scope, location, neighborhood) {
        return Err(EnterpriseError::AuthorityLocationMismatch {
            scope: authority.scope,
            location,
        });
    }
    // Supporting businesses are live organizational assets, not passive flavor tags. An active
    // racket locks their ownership and charges their recurring support cost, so binding one must
    // require the same authority coverage as binding the hosted location itself. Without this
    // gate a district manager could consume and lock infrastructure in another district.
    for business_id in supporting_businesses {
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        if !can_authority_cover_location(
            authority.scope,
            EnterpriseLocation::Business(*business_id),
            business.neighborhood(),
        ) {
            return Err(EnterpriseError::AuthoritySupportingBusinessMismatch {
                scope: authority.scope,
                business: *business_id,
            });
        }
    }
    Ok(())
}

pub(crate) fn can_authority_cover_location(
    scope: ResponsibilityScope,
    location: EnterpriseLocation,
    neighborhood: crate::core::id::NeighborhoodId,
) -> bool {
    match scope {
        ResponsibilityScope::Function(ResponsibilityFunction::Enterprise) => true,
        ResponsibilityScope::Function(
            ResponsibilityFunction::Territory
            | ResponsibilityFunction::Operations
            | ResponsibilityFunction::Intelligence
            | ResponsibilityFunction::Finance
            | ResponsibilityFunction::Legal
            | ResponsibilityFunction::Political
            | ResponsibilityFunction::Personnel,
        ) => false,
        ResponsibilityScope::Neighborhood(id) => id == neighborhood,
        ResponsibilityScope::Business(id) => {
            matches!(location, EnterpriseLocation::Business(location_id) if location_id == id)
        }
    }
}

pub(crate) fn resolve_location_neighborhood(
    state: &AppState,
    location: EnterpriseLocation,
) -> Result<crate::core::id::NeighborhoodId, EnterpriseError> {
    match location {
        EnterpriseLocation::Neighborhood(id) => {
            let _ = state
                .world
                .get_neighborhood(id)
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            Ok(id)
        }
        EnterpriseLocation::Business(id) => {
            let business = state
                .world
                .get_business(id)
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            let _ = state
                .world
                .get_neighborhood(business.neighborhood())
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            Ok(business.neighborhood())
        }
    }
}

pub(super) fn resolve_location_profile(
    state: &AppState,
    location: EnterpriseLocation,
) -> Result<NeighborhoodProfile, EnterpriseError> {
    let neighborhood = resolve_location_neighborhood(state, location)?;
    Ok(state
        .world
        .get_neighborhood(neighborhood)
        .expect("validated enterprise neighborhood must exist")
        .profile())
}

fn validate_business_location_requirements(
    definition: &EnterpriseDefinition,
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
) -> Result<(), EnterpriseError> {
    let EnterpriseLocation::Business(business_id) = location else {
        return Ok(());
    };
    let business = state
        .world
        .get_business(business_id)
        .ok_or(EnterpriseError::InvalidLocation(location))?;
    for function in definition.required_business_functions() {
        if !business.has_function(*function) {
            return Err(EnterpriseError::MissingBusinessFunction {
                business: business_id,
                function: *function,
            });
        }
    }
    // The hosting venue must remain organization-owned while the racket runs, just like the
    // support network: a racket cannot keep settling at a business the organization no longer owns.
    if business.owner() != BusinessOwner::Organization(organization) {
        return Err(EnterpriseError::HostBusinessOwnershipMismatch {
            business: business_id,
            owner: business.owner(),
            organization,
        });
    }
    Ok(())
}

pub(super) fn validate_enterprise_business_dependencies(
    definition: &EnterpriseDefinition,
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<(), EnterpriseError> {
    validate_business_location_requirements(definition, state, organization, location)?;
    validate_supporting_businesses(state, organization, location, supporting_businesses)?;

    if definition.required_network_functions().is_empty() {
        return Ok(());
    }
    let mut available = BTreeSet::new();
    if let EnterpriseLocation::Business(business_id) = location {
        let business = state
            .world
            .get_business(business_id)
            .ok_or(EnterpriseError::InvalidLocation(location))?;
        available.extend(business.functions().iter().copied());
    }
    for business_id in supporting_businesses {
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        available.extend(business.functions().iter().copied());
    }
    for function in definition.required_network_functions() {
        if !available.contains(function) {
            return Err(EnterpriseError::MissingNetworkFunction {
                function: *function,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_supporting_businesses(
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<(), EnterpriseError> {
    for business_id in supporting_businesses {
        if matches!(location, EnterpriseLocation::Business(location_id) if location_id == *business_id)
        {
            return Err(EnterpriseError::DuplicateSupportingLocation {
                business: *business_id,
            });
        }
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        if business.owner() != BusinessOwner::Organization(organization) {
            return Err(EnterpriseError::SupportingBusinessOwnershipMismatch {
                business: *business_id,
                owner: business.owner(),
                organization,
            });
        }
    }
    Ok(())
}

pub(super) fn snapshot_supporting_business_versions(
    state: &AppState,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<BTreeMap<BusinessId, u32>, EnterpriseError> {
    supporting_businesses
        .iter()
        .map(|business_id| {
            let business = state
                .world
                .get_business(*business_id)
                .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
            Ok((*business_id, business.version()))
        })
        .collect()
}

pub(super) fn validate_supporting_business_versions(
    state: &AppState,
    versions: &BTreeMap<BusinessId, u32>,
) -> Result<(), EnterpriseError> {
    for (business_id, expected) in versions {
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        if business.version() != *expected {
            return Err(EnterpriseError::StaleSupportingBusiness {
                business: *business_id,
                expected: *expected,
                found: business.version(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_enterprise_accounts(
    state: &AppState,
    organization: OrganizationId,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    current_enterprise: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    validate_cash_account_kind(state, organization, cash_account)?;
    let settlement = state
        .finance
        .get_account(settlement_account)
        .ok_or(EnterpriseError::MissingAccount(settlement_account))?;
    if settlement.owner() != FinancialOwner::Organization(organization) {
        return Err(EnterpriseError::AccountOwnerMismatch {
            account: settlement_account,
            organization,
        });
    }
    match settlement.kind() {
        AccountKind::Settlement => {}
        AccountKind::StreetCash
        | AccountKind::ConcealedCash
        | AccountKind::AccountedFunds
        | AccountKind::LegitimateOperating => {
            return Err(EnterpriseError::InvalidSettlementAccountKind(
                settlement_account,
            ));
        }
    }
    // Settlement-account exclusivity is permanent, including after the incumbent closes:
    // cycle history and provenance keep referencing the account, so reassigning it would
    // corrupt past settlements' ownership trail.
    if let Some(existing) = state
        .enterprises
        .get_by_settlement_account(settlement_account)
        && Some(existing.id()) != current_enterprise
    {
        return Err(EnterpriseError::SettlementAccountInUse {
            account: settlement_account,
            enterprise: existing.id(),
        });
    }
    Ok(())
}

/// The manager's cycle report to leadership. Heat-bearing cycles say why cost rose, while a
/// reportable drop to zero says the street surcharge cleared. This lets leadership observe both
/// escalation and recovery without leaking hidden case detail.
pub(super) fn build_cycle_report_summary(
    state: &crate::core::state::AppState,
    record: &crate::enterprises::EnterpriseRecord,
    economics: &EnterpriseCycleEconomics,
    drew_vice_attention: bool,
    plan_suspends: bool,
) -> String {
    let base = format!(
        "Enterprise cycle reported gross {}, operating cost {}",
        crate::finance::helpers::format_money_cents(economics.gross_revenue.cents()),
        crate::finance::helpers::format_money_cents(economics.operating_cost.cents()),
    );
    let heat = if economics.investigation_heat > Money::ZERO {
        format!(
            ", including a {} street surcharge while police work stays heavy in {}",
            crate::finance::helpers::format_money_cents(economics.investigation_heat.cents()),
            resolve_enterprise_district_name(state, record),
        )
    } else if economics
        .previous_investigation_heat
        .is_some_and(|previous| previous > Money::ZERO)
    {
        format!(
            ", with the prior street surcharge cleared as police work eased in {}",
            resolve_enterprise_district_name(state, record),
        )
    } else {
        String::new()
    };
    let vice = if drew_vice_attention {
        format!(
            " Vice officers were noticed watching {}; district enforcement attention is focusing on this racket.",
            resolve_enterprise_location_name(state, record),
        )
    } else {
        String::new()
    };
    format!(
        "{base}{heat}, net cash {}, and {}.{vice}{}",
        crate::finance::helpers::format_money_cents(economics.net_cash.cents()),
        crate::finance::helpers::describe_gross_variance(economics.variance_basis_points),
        if plan_suspends {
            " Repeated losses have suspended the racket pending a manual resumption.".to_owned()
        } else {
            String::new()
        },
    )
}

fn resolve_enterprise_district_name(
    state: &crate::core::state::AppState,
    record: &crate::enterprises::EnterpriseRecord,
) -> String {
    let neighborhood = match record.location() {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(business_id) => state
            .world
            .get_business(business_id)
            .expect("enterprise business location must reference a persisted business")
            .neighborhood(),
    };
    state
        .world
        .get_neighborhood(neighborhood)
        .expect("enterprise location must reference a persisted neighborhood")
        .name()
        .to_owned()
}

/// Active law-enforcement originated cases (operation exposure or enterprise vice attention)
/// targeting this neighborhood: the shared pressure signal behind street-heat surcharges and vice
/// attention. Case pressure follows the live case, not today's intake-priority winner; otherwise a
/// jurisdiction handoff would make an old bureau's still-active investigation disappear from the
/// street overnight.
pub(super) fn count_district_originated_cases(
    state: &crate::core::state::AppState,
    neighborhood: crate::core::id::NeighborhoodId,
) -> u32 {
    let count = state
        .legal
        .active_investigations()
        .filter(|investigation| {
            state
                .world
                .get_organization(investigation.owner())
                .expect("active investigation owner must reference a persisted organization")
                .kind()
                == OrganizationKind::LawEnforcement
                && investigation.origin().is_some()
                // Enterprise settlements at one simulation instant are peer events. A vice
                // inquiry opened or resumed by an earlier-settled racket this minute must not
                // retroactively tax a later peer cycle merely because stable scheduler order
                // committed the first record earlier. Legal activity that existed before this
                // enterprise phase, including operation-created casework earlier in the tick,
                // still counts immediately.
                && !enterprise_vice_inquiry_became_active_this_minute(state, investigation)
                && crate::operations::operation_execution::resolve_investigation_target_neighborhoods(
                state, investigation,
            )
            .contains(&neighborhood)
        })
        .count();
    // The authored probability saturates at 10_000 bp and monetary multiplication is checked, so
    // a pathological campaign with more live cases than u32 can represent should saturate this
    // bounded pressure count rather than wrap it back toward zero.
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn enterprise_vice_inquiry_became_active_this_minute(
    state: &crate::core::state::AppState,
    investigation: &crate::legal::InvestigationRecord,
) -> bool {
    investigation.evidence().iter().any(|evidence_id| {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .expect("investigation evidence index must reference persisted evidence");
        let Some(EntityRef::Enterprise(enterprise)) = evidence.origin() else {
            return false;
        };
        evidence.discovered_at() == state.now()
            && is_enterprise_vice_evidence(state, investigation, evidence, enterprise)
            && state
                .enterprises
                .latest_cycle(enterprise)
                .is_some_and(|cycle| {
                    cycle.occurred_at() == state.now() && cycle.drew_vice_attention()
                })
    })
}

/// Whether this racket already has an active dedicated inquiry under any authority. Jurisdiction
/// priority can change while an existing investigation remains owned by the authority that opened
/// it, so restricting this lookup to the district's current intake authority would allow a second
/// simultaneous inquiry after a jurisdiction handoff. District pressure still counts the existing
/// case for heat; this query only prevents the visibility loop from manufacturing duplicates.
pub(super) fn has_active_enterprise_inquiry(
    state: &crate::core::state::AppState,
    enterprise: EnterpriseId,
) -> bool {
    state.legal.active_investigations().any(|investigation| {
        investigation.evidence().iter().any(|evidence_id| {
            let evidence = state
                .legal
                .get_evidence(*evidence_id)
                .expect("investigation evidence index must reference persisted evidence");
            is_enterprise_vice_evidence(state, investigation, evidence, enterprise)
        })
    })
}

/// Canonical persisted signature of enterprise vice intake. Incident continuation may resume a
/// shelf whose original case origin names some other overlapping incident, so the investigation's
/// `origin` alone is not reliable provenance for later vice-cycle behavior. The intake evidence is:
/// every vice event adds one enterprise-specific surveillance item even when it reuses a shelf.
fn is_enterprise_vice_evidence(
    state: &crate::core::state::AppState,
    investigation: &crate::legal::InvestigationRecord,
    evidence: &crate::legal::EvidenceRecord,
    enterprise: EnterpriseId,
) -> bool {
    let Some(_record) = state.enterprises.get_enterprise(enterprise) else {
        return false;
    };
    state
        .world
        .get_organization(investigation.owner())
        .is_some_and(|owner| owner.kind() == OrganizationKind::LawEnforcement)
        && investigation
            .subjects()
            .contains(&EntityRef::Enterprise(enterprise))
        && evidence.investigation() == investigation.id()
        && evidence.custodian() == investigation.owner()
        && evidence.subject() == EntityRef::Enterprise(enterprise)
        && evidence.origin() == Some(EntityRef::Enterprise(enterprise))
        && evidence.source().is_none()
        && evidence.kind() == crate::legal::EvidenceKind::Surveillance
        && evidence.strength() == crate::legal::EvidenceStrength::Weak
        && evidence.reliability() == crate::legal::EvidenceReliability::Questionable
        && evidence.admissibility() == crate::legal::Admissibility::Unknown
}

/// Builds the intake draft for a vice inquiry opened onto this racket: one questionable
/// surveillance item against the enterprise itself, originated by the enterprise so the
/// district heat loop and cold-case decay treat it exactly like any other street case.
pub(super) fn build_vice_incident_draft(
    state: &crate::core::state::AppState,
    enterprise: EnterpriseId,
    record: &crate::enterprises::EnterpriseRecord,
    owner: OrganizationId,
    discovered_at: crate::core::time::SimTime,
) -> crate::legal::IncidentIntakeDraft {
    use crate::core::entity::EntityRef;
    let location_name = resolve_enterprise_location_name(state, record);
    crate::legal::IncidentIntakeDraft {
        owner,
        title: format!("Vice inquiry into {location_name}"),
        subjects: BTreeSet::from([EntityRef::Enterprise(record.id())]),
        evidence: vec![crate::legal::IncidentEvidenceDraft {
            subject: EntityRef::Enterprise(record.id()),
            origin: Some(EntityRef::Enterprise(record.id())),
            kind: crate::legal::EvidenceKind::Surveillance,
            strength: crate::legal::EvidenceStrength::Weak,
            reliability: crate::legal::EvidenceReliability::Questionable,
            admissibility: crate::legal::Admissibility::Unknown,
            discovered_at,
        }],
        origin: Some(EntityRef::Enterprise(enterprise)),
        witness: None,
    }
}

/// Human-facing venue description used in vice-inquiry titles.
fn resolve_enterprise_location_name(
    state: &crate::core::state::AppState,
    record: &crate::enterprises::EnterpriseRecord,
) -> String {
    match record.location() {
        EnterpriseLocation::Business(business_id) => state
            .world
            .get_business(business_id)
            .expect("enterprise business location must reference a persisted business")
            .name()
            .to_owned(),
        EnterpriseLocation::Neighborhood(_) => resolve_enterprise_district_name(state, record),
    }
}
