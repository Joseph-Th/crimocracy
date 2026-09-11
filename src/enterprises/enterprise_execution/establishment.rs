//! Enterprise establishment validation and atomic commit.

use super::*;

pub struct ValidatedEnterpriseEstablishment {
    draft: EnterpriseDraft,
    authority: ResolvedMandateAuthority,
    cycle_duration: SimDuration,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
    /// The venue-hosting business's version at validation time; commit re-checks it so a
    /// host that changed hands or lost a required function between phases cannot host.
    host_business_version: Option<(BusinessId, u32)>,
    /// Present only for atomic autonomous establishment that must open a fresh settlement
    /// account. The account is planned read-only and opened only after every enterprise
    /// dependency has been revalidated.
    account_openings: Option<ValidatedFinancialAccountOpenings>,
}

impl ValidatedEnterpriseEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<EnterpriseId, EnterpriseError> {
        ensure_mandate_authority_current(state, self.authority)?;
        validate_enterprise_environment(
            state,
            self.draft.organization,
            self.draft.authority,
            self.draft.location,
            &self.draft.supporting_businesses,
        )?;
        // One racket of a kind per spot is re-checked at commit: a second token validated
        // before an identical establishment committed (or held across one) must reject here
        // rather than double-book the location.
        if enterprise_location_is_occupied(state, self.draft.kind, self.draft.location) {
            return Err(EnterpriseError::DuplicateEnterpriseAtLocation {
                kind: self.draft.kind,
                location: self.draft.location,
            });
        }
        if let Some((business_id, expected)) = self.host_business_version {
            // The host passed venue validation at validate time; any change of ownership or
            // function bumps the business version, so re-checking the pinned version is enough.
            let business =
                state
                    .world
                    .get_business(business_id)
                    .ok_or(EnterpriseError::InvalidLocation(
                        EnterpriseLocation::Business(business_id),
                    ))?;
            if business.version() != expected {
                return Err(EnterpriseError::StaleHostBusiness {
                    business: business_id,
                    expected,
                    found: business.version(),
                });
            }
        }
        validate_supporting_business_versions(state, &self.supporting_business_versions)?;
        validate_supporting_businesses(
            state,
            self.draft.organization,
            self.draft.location,
            &self.draft.supporting_businesses,
        )?;
        match &self.account_openings {
            Some(openings) => validate_enterprise_accounts_with_planned_settlement(
                state,
                self.draft.organization,
                self.draft.cash_account,
                self.draft.settlement_account,
                openings,
            )?,
            None => validate_enterprise_accounts(
                state,
                self.draft.organization,
                self.draft.cash_account,
                self.draft.settlement_account,
                None,
            )?,
        }
        let established_at = state.now();
        let next_cycle_at = established_at
            .checked_add(self.cycle_duration)
            .ok_or(EnterpriseError::SimulationTimeOverflow)?;
        let id = match self.account_openings {
            Some(openings) => {
                state.ids.reserve_many(&[
                    (
                        IdKind::FinancialAccount,
                        u32::try_from(openings.len())
                            .expect("validated enterprise account-opening count must fit u32"),
                    ),
                    (IdKind::Enterprise, 1),
                ])?;
                openings.commit(state)?;
                state
                    .ids
                    .next_enterprise()
                    .expect("composite enterprise ID preflight must make allocation infallible")
            }
            None => state.ids.next_enterprise()?,
        };
        state.enterprises.insert(build_enterprise_record(
            id,
            self.draft,
            established_at,
            next_cycle_at,
        ));
        Ok(id)
    }
}

pub fn validate_establish_enterprise(
    registry: &Registry,
    state: &AppState,
    draft: EnterpriseDraft,
) -> Result<ValidatedEnterpriseEstablishment, EnterpriseError> {
    validate_establish_enterprise_with_optional_openings(registry, state, draft, None)
}

pub(crate) fn validate_establish_enterprise_with_openings(
    registry: &Registry,
    state: &AppState,
    draft: EnterpriseDraft,
    openings: ValidatedFinancialAccountOpenings,
) -> Result<ValidatedEnterpriseEstablishment, EnterpriseError> {
    if openings.len() != 1
        || !openings.account_matches(
            draft.settlement_account,
            FinancialOwner::Organization(draft.organization),
            AccountKind::Settlement,
        )
    {
        return Err(EnterpriseError::MissingAccount(draft.settlement_account));
    }
    validate_establish_enterprise_with_optional_openings(registry, state, draft, Some(openings))
}

fn validate_establish_enterprise_with_optional_openings(
    registry: &Registry,
    state: &AppState,
    draft: EnterpriseDraft,
    account_openings: Option<ValidatedFinancialAccountOpenings>,
) -> Result<ValidatedEnterpriseEstablishment, EnterpriseError> {
    let definition = registry.get_enterprise(draft.kind);
    let authority = resolve_mandate_authority(state, draft.authority)?;
    validate_enterprise_environment(
        state,
        draft.organization,
        draft.authority,
        draft.location,
        &draft.supporting_businesses,
    )?;
    // Active and suspended rackets reserve their spot. A terminally retired record remains
    // historical truth but no longer blocks a genuinely new racket from being established.
    if enterprise_location_is_occupied(state, draft.kind, draft.location) {
        return Err(EnterpriseError::DuplicateEnterpriseAtLocation {
            kind: draft.kind,
            location: draft.location,
        });
    }
    validate_enterprise_business_dependencies(
        definition,
        state,
        draft.organization,
        draft.location,
        &draft.supporting_businesses,
    )?;
    match &account_openings {
        Some(openings) => validate_enterprise_accounts_with_planned_settlement(
            state,
            draft.organization,
            draft.cash_account,
            draft.settlement_account,
            openings,
        )?,
        None => validate_enterprise_accounts(
            state,
            draft.organization,
            draft.cash_account,
            draft.settlement_account,
            None,
        )?,
    }
    let cycle_duration = definition.economics().cycle();
    state
        .now()
        .checked_add(cycle_duration)
        .ok_or(EnterpriseError::SimulationTimeOverflow)?;
    let supporting_business_versions =
        snapshot_supporting_business_versions(state, &draft.supporting_businesses)?;
    let host_business_version = match draft.location {
        EnterpriseLocation::Business(business_id) => {
            let business = state
                .world
                .get_business(business_id)
                .ok_or(EnterpriseError::InvalidLocation(draft.location))?;
            Some((business_id, business.version()))
        }
        EnterpriseLocation::Neighborhood(_) => None,
    };
    Ok(ValidatedEnterpriseEstablishment {
        draft,
        authority,
        cycle_duration,
        supporting_business_versions,
        host_business_version,
        account_openings,
    })
}

pub(crate) fn enterprise_location_is_occupied(
    state: &AppState,
    kind: EnterpriseKind,
    location: EnterpriseLocation,
) -> bool {
    state
        .enterprises()
        .enterprises_at(location)
        .any(|record| record.kind() == kind && record.status() != EnterpriseStatus::Retired)
}
