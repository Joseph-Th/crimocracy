//! Automatic-policy legal-support orchestration.
//!
//! Direct retain/end commands remain owned by the parent legal-representation system. This child
//! module owns only the deterministic governance pass that selects, funds, and concludes automatic
//! policy retainers through those canonical commands.

use super::{
    LegalRepresentationError, ValidatedLegalRepresentation, validate_end_legal_representation,
    validate_retain_legal_representation,
};
use crate::contacts::{ContactKind, ContactStatus};
use crate::core::id::{
    ArrestId, CharacterId, ContactId, FinancialAccountId, LegalRepresentationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    DelegationError, PolicySource, resolve_policy_for_manager,
};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::resolve_budget_usage;
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::legal::{ArrestStatus, LegalRepresentationDraft, LegalRepresentationEndReason};
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
struct AutomaticLegalSupportCandidate {
    arrest: ArrestId,
    sponsor: OrganizationId,
    authorization: Option<MandateAuthority>,
}

#[derive(Clone, Copy, Debug)]
struct ResolvedAutomaticLegalSupport {
    authorization: Option<MandateAuthority>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutomaticLegalSupportOutcome {
    pub(crate) retained: Vec<LegalRepresentationId>,
    pub(crate) concluded: usize,
}

/// Executes `AssociateLegalSupport(Automatic)` governance: every detained member of an
/// organization that runs the Automatic policy gets counsel retained through the canonical
/// representation path. Organization policy may aggregate sponsor liquidity; a mandate-sourced
/// override must spend through its Legal-scope budget account and current budget window. Counsel
/// is selected from the strongest currently usable LegalServices channel by LegalKnowledge,
/// with stable contact/account IDs only as exact-skill tie-breakers.
/// Organizations without those prerequisites see no action — the policy promises support, and
/// this stage delivers it only when the pieces exist for the canonical transaction to carry it.
pub(crate) fn apply_automatic_legal_support(
    registry: &crate::registry::Registry,
    state: &mut AppState,
) -> Result<AutomaticLegalSupportOutcome, LegalRepresentationError> {
    // Automatic support concludes when the matter it covers does: a representation this pass
    // retained for a detainee who has left custody ends with `MatterConcluded`, so concluded
    // matters stop blocking contact termination. A contact may carry several concurrent
    // matters — counsel serves multiple clients — but it cannot be terminated while any of
    // them is still active.
    // Explicitly commanded retentions are never swept here — their matter is leadership's
    // to end, not governance's.
    //
    // Both halves of this stage need custody work to observe: the sweep ends automatic
    // representations whose detainee left, and retention needs a detained member. With no
    // representation and nobody in custody there is nothing this pass could do, so quiet
    // ticks skip both scans.
    if !state.legal.has_active_automatic_policy_representations()
        && !state.legal.has_detained_arrests()
    {
        return Ok(AutomaticLegalSupportOutcome::default());
    }
    let concluded = conclude_inactive_automatic_representations(state)?;
    let candidates = resolve_automatic_legal_support_candidates(state)?;
    let mut retained = Vec::new();
    for candidate in candidates {
        let fee = registry.legal().automatic_support_retainer();
        let Some(payer_accounts) = resolve_automatic_support_payer_accounts(state, candidate, fee)?
        else {
            continue;
        };
        let Some(validated_representation) =
            validate_best_usable_automatic_counsel(state, candidate, fee, &payer_accounts)?
        else {
            continue;
        };
        retained.push(validated_representation.commit(state)?);
    }
    Ok(AutomaticLegalSupportOutcome {
        retained,
        concluded,
    })
}

fn conclude_inactive_automatic_representations(
    state: &mut AppState,
) -> Result<usize, LegalRepresentationError> {
    let concluded: Vec<LegalRepresentationId> = state
        .legal
        .active_automatic_policy_representations()
        .filter(|record| {
            state
                .legal
                .get_arrest(record.arrest())
                .is_none_or(|arrest| arrest.status() != ArrestStatus::Detained)
        })
        .map(|record| record.id())
        .collect();
    let concluded_count = concluded.len();
    if concluded_count == 0 {
        return Ok(0);
    }

    // Validate every ending before the first representation mutates. Each ending emits one
    // information record and one report; reserve the whole batch up front so allocator
    // exhaustion cannot conclude a prefix of same-minute automatic matters.
    let endings = concluded
        .into_iter()
        .map(|representation| {
            validate_end_legal_representation(
                state,
                representation,
                LegalRepresentationEndReason::MatterConcluded,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let artifact_budget = endings
        .iter()
        .flat_map(
            crate::legal::legal_representation_system::ValidatedLegalRepresentationEnd::id_budget,
        )
        .collect::<Vec<_>>();
    state.ids.reserve_many(&artifact_budget)?;
    for ending in endings {
        ending.commit_preflighted(state);
    }
    Ok(concluded_count)
}

fn resolve_automatic_legal_support_candidates(
    state: &AppState,
) -> Result<Vec<AutomaticLegalSupportCandidate>, LegalRepresentationError> {
    let mut candidates = Vec::new();
    for arrest in state.legal.detained_arrests() {
        if state
            .legal
            .active_representation_for_arrest(arrest.id())
            .is_some()
        {
            continue;
        }
        let defendant = arrest.character();
        let defendant_record = state
            .world
            .get_character(defendant)
            .ok_or(LegalRepresentationError::MissingDefendant(defendant))?;
        let Some(organization) = defendant_record.organization() else {
            continue;
        };
        let record = state
            .world
            .get_organization(organization)
            .ok_or(LegalRepresentationError::MissingSponsor(organization))?;
        if record.kind() != OrganizationKind::Criminal {
            continue;
        }
        let Some(resolved) =
            resolve_automatic_support_for_defendant(state, defendant_record, record)?
        else {
            continue;
        };
        candidates.push(AutomaticLegalSupportCandidate {
            arrest: arrest.id(),
            sponsor: organization,
            authorization: resolved.authorization,
        });
    }
    Ok(candidates)
}

fn resolve_automatic_support_for_defendant(
    state: &AppState,
    defendant: &crate::world::CharacterRecord,
    sponsor: &crate::world::OrganizationRecord,
) -> Result<Option<ResolvedAutomaticLegalSupport>, LegalRepresentationError> {
    use crate::world::{LegalSupportPolicy, PolicyKind, PolicySetting};

    // The supervisor's standing order governs first. A detained supervisor cannot exercise
    // delegated authority, so the organization's standing default takes over. Every other
    // delegation error denotes malformed current governance and must surface rather than be
    // disguised as an ordinary policy fallback.
    let (setting, authorization) = match defendant.supervisor() {
        None => (sponsor.policy(PolicyKind::AssociateLegalSupport), None),
        Some(supervisor) => {
            match resolve_policy_for_manager(state, supervisor, PolicyKind::AssociateLegalSupport) {
                Ok(resolved) => {
                    let authorization = match resolved.source {
                        PolicySource::Organization(_) => None,
                        PolicySource::Mandate(mandate) => {
                            let Some(authority) = resolve_automatic_legal_support_authority(
                                state, supervisor, mandate,
                            ) else {
                                return Ok(None);
                            };
                            Some(authority)
                        }
                    };
                    (Some(resolved.setting), authorization)
                }
                Err(DelegationError::DetainedManager { .. }) => {
                    (sponsor.policy(PolicyKind::AssociateLegalSupport), None)
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    if !matches!(
        setting,
        Some(PolicySetting::AssociateLegalSupport(
            LegalSupportPolicy::Automatic
        )),
    ) {
        return Ok(None);
    }
    Ok(Some(ResolvedAutomaticLegalSupport { authorization }))
}

fn resolve_automatic_legal_support_authority(
    state: &AppState,
    supervisor: CharacterId,
    mandate: crate::core::id::MandateId,
) -> Option<MandateAuthority> {
    let legal_scope = ResponsibilityScope::Function(ResponsibilityFunction::Legal);
    let mandate_record = state
        .delegation
        .get_mandate(mandate)
        .expect("resolved mandate policy must reference a live mandate");
    // Delegation already guarantees that an AssociateLegalSupport standing order carries
    // Legal scope. Legal retention adds the distinct spending prerequisite: a mandate-sourced
    // automatic policy still needs an explicit budget.
    mandate_record.budget()?;
    Some(MandateAuthority {
        mandate,
        manager: supervisor,
        scope: legal_scope,
    })
}

fn resolve_automatic_support_payer_accounts(
    state: &AppState,
    candidate: AutomaticLegalSupportCandidate,
    fee: Money,
) -> Result<Option<BTreeSet<FinancialAccountId>>, LegalRepresentationError> {
    // Organization policy can draw across the sponsor's liquid reserves. A mandate-sourced
    // policy must instead use exactly its configured budget account and stay inside the
    // current budget window, matching explicit delegated retention.
    let payer_accounts: BTreeSet<_> = if let Some(authority) = candidate.authorization {
        let mandate = state
            .delegation
            .get_mandate(authority.mandate)
            .expect("automatic delegated policy must retain its mandate");
        let budget = mandate
            .budget()
            .expect("automatic delegated support candidate requires a mandate budget");
        let usage = resolve_budget_usage(state, authority.mandate, state.now())?;
        let funding = state
            .finance
            .get_account(budget.funding_account)
            .expect("validated mandate budget account must persist");
        if usage.remaining < fee || funding.spendable_balance() < fee {
            return Ok(None);
        }
        BTreeSet::from([budget.funding_account])
    } else {
        state
            .finance
            .accounts_for(FinancialOwner::Organization(candidate.sponsor))
            .filter(|account| account.spendable_balance() > Money::ZERO)
            .map(|account| account.id())
            .collect()
    };
    let available = payer_accounts
        .iter()
        .map(|account| {
            state
                .finance
                .get_account(*account)
                .expect("funding account came from finance owner index")
                .spendable_balance()
                .cents()
        })
        .fold(0_i128, |total, cents| {
            (total + i128::from(cents)).min(i128::from(fee.cents()))
        });
    if available < i128::from(fee.cents()) {
        return Ok(None);
    }
    Ok(Some(payer_accounts))
}

fn validate_best_usable_automatic_counsel(
    state: &AppState,
    candidate: AutomaticLegalSupportCandidate,
    fee: Money,
    payer_accounts: &BTreeSet<FinancialAccountId>,
) -> Result<Option<ValidatedLegalRepresentation>, LegalRepresentationError> {
    // Legal contacts are broader than retained counsel: prosecutors and legal authorities also
    // expose Legal channels. Rank every currently viable LegalServices lawyer by actual legal
    // competence rather than contact creation order. Contact/account IDs are deterministic
    // tie-breakers only, so adding a stronger later relationship can improve automatic defense.
    let mut best: Option<(Reverse<u8>, ContactId, FinancialAccountId)> = None;
    for contact in state.contacts.contacts_for_sponsor(candidate.sponsor) {
        if contact.status() != ContactStatus::Active
            || contact.kind() != ContactKind::Legal
            || !crate::contacts::contact_system::are_channel_endpoints_available(state, contact)
            || !crate::contacts::contact_system::has_current_contact_relationship_basis(
                state, contact,
            )
        {
            continue;
        }
        let institution = state.world.get_organization(contact.institution()).ok_or(
            LegalRepresentationError::MissingCounselInstitution(contact.institution()),
        )?;
        if institution.kind() != OrganizationKind::LegalServices {
            continue;
        }
        let counsel = state
            .world
            .get_character(contact.contact())
            .ok_or(LegalRepresentationError::MissingCounsel(contact.contact()))?;
        let Some(legal_knowledge) = counsel.capability(CapabilityKind::LegalKnowledge) else {
            continue;
        };
        let Some(provider_account) = state
            .finance
            .accounts_for(FinancialOwner::Organization(contact.institution()))
            .find(|account| account.kind() == AccountKind::LegitimateOperating)
        else {
            continue;
        };
        let ranked = (
            Reverse(legal_knowledge.value()),
            contact.id(),
            provider_account.id(),
        );
        if best.is_none_or(|current| ranked < current) {
            best = Some(ranked);
        }
    }
    let Some((_, contact, provider_account)) = best else {
        return Ok(None);
    };
    validate_retain_legal_representation(
        state,
        LegalRepresentationDraft {
            arrest: candidate.arrest,
            sponsor: candidate.sponsor,
            contact,
            fee,
            payer_accounts: payer_accounts.clone(),
            provider_account,
            authorization: candidate.authorization,
            origin: crate::legal::LegalRepresentationOrigin::AutomaticPolicy,
        },
    )
    .map(Some)
}
