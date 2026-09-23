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
    ArrestId, CharacterId, ContactId, FinancialAccountId, IdKind, LegalRepresentationId, MandateId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance_by};
use crate::delegation::delegation_system::{
    DelegationError, PolicySource, resolve_policy_for_manager,
};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{FinanceError, resolve_budget_usage};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::legal::{ArrestStatus, LegalRepresentationDraft, LegalRepresentationEndReason};
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Clone, Copy, Debug)]
struct AutomaticCounselSelection {
    contact: ContactId,
    provider_account: FinancialAccountId,
}

#[derive(Clone, Debug)]
struct PlannedAutomaticLegalSupportRetention {
    candidate: AutomaticLegalSupportCandidate,
    payer_accounts: BTreeSet<FinancialAccountId>,
    counsel: AutomaticCounselSelection,
}

#[derive(Debug, Default)]
struct AutomaticSupportFinanceProjection {
    balances: BTreeMap<FinancialAccountId, Money>,
    account_advances: BTreeMap<FinancialAccountId, u32>,
    mandate_spending: BTreeMap<MandateId, Money>,
}

struct AutomaticLegalSupportRetentionPlan {
    retentions: Vec<PlannedAutomaticLegalSupportRetention>,
    id_budget: Vec<(IdKind, u32)>,
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

    // Treat the whole governance pass as one fallible cohort. Concluding old matters and retaining
    // new counsel can both allocate artifacts, while several retainers can share sponsor cash,
    // provider accounts, or a delegated budget. Plan the exact sequential funding outcome and
    // reserve its complete ID/version capacity before the first representation changes.
    let endings = validate_inactive_automatic_representation_endings(state)?;
    let concluded = endings.len();
    let retention_plan = plan_automatic_legal_support_retentions(registry, state)?;
    let mut id_budget = retention_plan.id_budget;
    id_budget.extend(endings.iter().flat_map(
        crate::legal::legal_representation_system::ValidatedLegalRepresentationEnd::id_budget,
    ));
    if state.ids.reserve_many(&id_budget).is_err() {
        // Automatic support is one fully planned governance cohort. At a finite persistence rail,
        // conclude and retain nothing rather than letting a canonical tick fail after earlier
        // phases have committed. Direct retention/end commands keep their typed exhaustion errors.
        return Ok(AutomaticLegalSupportOutcome::default());
    }

    for ending in endings {
        ending.commit_preflighted(state);
    }

    let fee = registry.legal().automatic_support_retainer();
    let mut retained = Vec::with_capacity(retention_plan.retentions.len());
    for planned in retention_plan.retentions {
        let validated = validate_automatic_counsel_retention(
            state,
            planned.candidate,
            fee,
            &planned.payer_accounts,
            planned.counsel,
        )
        .expect("preflighted automatic legal-support retention must remain valid within one pass");
        retained.push(
            validated
                .commit(state)
                .expect("preflighted automatic legal-support retention must commit"),
        );
    }
    Ok(AutomaticLegalSupportOutcome {
        retained,
        concluded,
    })
}

fn validate_inactive_automatic_representation_endings(
    state: &AppState,
) -> Result<Vec<super::ValidatedLegalRepresentationEnd>, LegalRepresentationError> {
    let mut concluded = Vec::new();
    for record in state.legal.active_automatic_policy_representations() {
        let arrest = state
            .legal
            .get_arrest(record.arrest())
            .ok_or(LegalRepresentationError::MissingArrest(record.arrest()))?;
        if arrest.status() != ArrestStatus::Detained {
            concluded.push(record.id());
        }
    }
    concluded
        .into_iter()
        .map(|representation| {
            validate_end_legal_representation(
                state,
                representation,
                LegalRepresentationEndReason::MatterConcluded,
            )
        })
        .collect()
}

fn plan_automatic_legal_support_retentions(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<AutomaticLegalSupportRetentionPlan, LegalRepresentationError> {
    let candidates = resolve_automatic_legal_support_candidates(state)?;
    let fee = registry.legal().automatic_support_retainer();
    let mut projection = AutomaticSupportFinanceProjection::default();
    let mut retentions = Vec::new();
    let mut id_budget = Vec::new();

    for candidate in candidates {
        let Some(payer_accounts) =
            resolve_automatic_support_payer_accounts(state, candidate, fee, &projection)?
        else {
            continue;
        };
        let Some(counsel) =
            resolve_best_usable_automatic_counsel(state, candidate, fee, &projection)?
        else {
            continue;
        };

        // Validate every non-projected dependency now. The finance projection below then proves
        // the cumulative balance/version/budget effects that single-record validation cannot see
        // while the real state is intentionally still untouched.
        let validated =
            validate_automatic_counsel_retention(state, candidate, fee, &payer_accounts, counsel)?;
        projection.plan_retainer(
            state,
            candidate,
            fee,
            &payer_accounts,
            counsel.provider_account,
        )?;
        id_budget.extend(validated.id_budget());
        retentions.push(PlannedAutomaticLegalSupportRetention {
            candidate,
            payer_accounts,
            counsel,
        });
    }

    Ok(AutomaticLegalSupportRetentionPlan {
        retentions,
        id_budget,
    })
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
    projection: &AutomaticSupportFinanceProjection,
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
        let already_planned = projection
            .mandate_spending
            .get(&authority.mandate)
            .copied()
            .unwrap_or(Money::ZERO);
        let remaining = usage
            .remaining
            .checked_sub(already_planned)
            .ok_or(FinanceError::BudgetOverflow(authority.mandate))?;
        let funding = state
            .finance
            .get_account(budget.funding_account)
            .expect("validated mandate budget account must persist");
        if remaining < fee || projection.spendable_balance(funding) < fee {
            return Ok(None);
        }
        BTreeSet::from([budget.funding_account])
    } else {
        state
            .finance
            .accounts_for(FinancialOwner::Organization(candidate.sponsor))
            .filter(|account| projection.spendable_balance(account) > Money::ZERO)
            .map(|account| account.id())
            .collect()
    };
    let available = payer_accounts
        .iter()
        .map(|account| {
            let record = state
                .finance
                .get_account(*account)
                .expect("funding account came from finance owner index");
            projection.spendable_balance(record).cents()
        })
        .fold(0_i128, |total, cents| {
            (total + i128::from(cents)).min(i128::from(fee.cents()))
        });
    if available < i128::from(fee.cents()) {
        return Ok(None);
    }
    Ok(Some(payer_accounts))
}

impl AutomaticSupportFinanceProjection {
    fn account_has_posting_headroom(
        &self,
        account: &crate::finance::FinancialAccountRecord,
    ) -> bool {
        automatic_account_has_posting_headroom(
            account.version(),
            self.account_advances
                .get(&account.id())
                .copied()
                .unwrap_or(0),
        )
    }

    fn balance(&self, account: &crate::finance::FinancialAccountRecord) -> Money {
        self.balances
            .get(&account.id())
            .copied()
            .unwrap_or_else(|| account.balance())
    }

    fn spendable_balance(&self, account: &crate::finance::FinancialAccountRecord) -> Money {
        if !self.account_has_posting_headroom(account) {
            return Money::ZERO;
        }
        let balance = self.balance(account);
        if account.kind().is_liquid() && balance > Money::ZERO {
            balance
        } else {
            Money::ZERO
        }
    }

    fn plan_retainer(
        &mut self,
        state: &AppState,
        candidate: AutomaticLegalSupportCandidate,
        fee: Money,
        payer_accounts: &BTreeSet<FinancialAccountId>,
        provider_account: FinancialAccountId,
    ) -> Result<(), LegalRepresentationError> {
        let mut payers = Vec::with_capacity(payer_accounts.len());
        for account_id in payer_accounts {
            let account = state
                .finance
                .get_account(*account_id)
                .ok_or(LegalRepresentationError::MissingAccount(*account_id))?;
            let spendable = self.spendable_balance(account);
            if spendable > Money::ZERO {
                payers.push((
                    account.kind().unrestricted_spending_priority(),
                    spendable,
                    account.id(),
                ));
            }
        }
        payers.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(right.1.cmp(&left.1))
                .then(left.2.cmp(&right.2))
        });

        let mut remaining = fee;
        for (_, spendable, account) in payers {
            if remaining == Money::ZERO {
                break;
            }
            let debit = spendable.min(remaining);
            let debit = debit
                .checked_neg()
                .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?;
            self.apply_posting(state, account, debit)?;
            remaining = remaining
                .checked_add(debit)
                .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?;
        }
        if remaining != Money::ZERO {
            let available = fee
                .checked_sub(remaining)
                .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?;
            return Err(LegalRepresentationError::InsufficientFunds {
                available_cents: available.cents(),
                required_cents: fee.cents(),
            });
        }
        self.apply_posting(state, provider_account, fee)?;

        if let Some(authority) = candidate.authorization {
            let already_planned = self
                .mandate_spending
                .get(&authority.mandate)
                .copied()
                .unwrap_or(Money::ZERO);
            let next_planned = already_planned
                .checked_add(fee)
                .expect("delegated payer resolution proved the complete projected budget headroom");
            self.mandate_spending
                .insert(authority.mandate, next_planned);
        }
        Ok(())
    }

    fn apply_posting(
        &mut self,
        state: &AppState,
        account: FinancialAccountId,
        amount: Money,
    ) -> Result<(), LegalRepresentationError> {
        let record = state
            .finance
            .get_account(account)
            .ok_or(LegalRepresentationError::MissingAccount(account))?;
        let current = self.balance(record);
        let balance = current
            .checked_add(amount)
            .ok_or(FinanceError::BalanceOverflow(account))?;
        let advances = self.account_advances.entry(account).or_insert(0);
        *advances = advances
            .checked_add(1)
            .ok_or_else(|| VersionCapacityError::new("financial account"))?;
        ensure_version_can_advance_by(record.version(), *advances, "financial account")?;
        self.balances.insert(account, balance);
        Ok(())
    }
}

pub(super) fn automatic_account_has_posting_headroom(version: u32, planned_advances: u32) -> bool {
    version
        .checked_add(planned_advances)
        .is_some_and(|projected_version| projected_version < u32::MAX)
}

fn resolve_best_usable_automatic_counsel(
    state: &AppState,
    candidate: AutomaticLegalSupportCandidate,
    fee: Money,
    projection: &AutomaticSupportFinanceProjection,
) -> Result<Option<AutomaticCounselSelection>, LegalRepresentationError> {
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
            .filter(|account| account.kind() == AccountKind::LegitimateOperating)
            .filter(|account| projection.account_has_posting_headroom(account))
            .filter(|account| projection.balance(account).checked_add(fee).is_some())
            .min_by_key(|account| account.id())
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
    Ok(Some(AutomaticCounselSelection {
        contact,
        provider_account,
    }))
}

fn validate_automatic_counsel_retention(
    state: &AppState,
    candidate: AutomaticLegalSupportCandidate,
    fee: Money,
    payer_accounts: &BTreeSet<FinancialAccountId>,
    counsel: AutomaticCounselSelection,
) -> Result<ValidatedLegalRepresentation, LegalRepresentationError> {
    validate_retain_legal_representation(
        state,
        LegalRepresentationDraft {
            arrest: candidate.arrest,
            sponsor: candidate.sponsor,
            contact: counsel.contact,
            fee,
            payer_accounts: payer_accounts.clone(),
            provider_account: counsel.provider_account,
            authorization: candidate.authorization,
            origin: crate::legal::LegalRepresentationOrigin::AutomaticPolicy,
        },
    )
}
