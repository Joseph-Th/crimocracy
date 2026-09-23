//! Daily organizational payroll: canonical wage charges, shortfall resentment, and reporting.
//!
//! Payroll is the organization's standing carrying cost. It runs at every campaign-day boundary
//! through the same production paths a player-driven payment would use: money moves only through
//! validated ledger transactions, relationship damage only through the canonical relationship
//! path, and player-visible consequences only through persisted reports. No random stream is
//! consumed, so payroll never perturbs any domain RNG sequence.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, FinancialAccountId, IdExhaustionError, IdKind, LedgerTransactionId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime};
use crate::finance::finance_system::{
    FinanceError, ValidatedFinancialAccountOpenings, ValidatedLedgerTransaction,
    validate_open_accounts, validate_record_transaction, validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money, helpers::format_money_cents,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::social::RelationshipDimensions;
use crate::social::relationship_system::{
    RelationshipError, ValidatedRelationship, validate_set_relationship,
};
use crate::world::OrganizationKind;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum PayrollError {
    #[error("payroll member count exceeds supported range")]
    MemberCountOverflow,
    #[error("payroll arithmetic overflowed")]
    ArithmeticOverflow,
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Relationship(#[from] RelationshipError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

/// One organization's resolved payroll run for a single campaign day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayrollOutcome {
    organization: OrganizationId,
    owed: Money,
    paid: Money,
    short: Money,
    transaction: Option<LedgerTransactionId>,
}

struct OrganizationPayrollPlan {
    organization: OrganizationId,
    funding: Vec<FinancialAccountId>,
    per_member: Money,
    owed: Money,
    paid: Money,
    allocations: Vec<(CharacterId, Option<CharacterId>, Money)>,
}

impl OrganizationPayrollPlan {
    fn short(&self) -> Money {
        self.owed
            .checked_sub(self.paid)
            .expect("planned payroll payment cannot exceed the wage obligation")
    }

    fn id_budget(&self, state: &AppState) -> Vec<(IdKind, u32)> {
        let mut budget = Vec::new();
        if self.paid > Money::ZERO {
            let missing_wage_accounts = self
                .allocations
                .iter()
                .filter(|(member, _, amount)| {
                    *amount > Money::ZERO
                        && find_existing_wage_account(state, *member, *amount).is_none()
                })
                .count();
            budget.push((
                IdKind::FinancialAccount,
                u32::try_from(missing_wage_accounts)
                    .expect("payroll member count must fit the financial-account ID space"),
            ));
            budget.push((IdKind::LedgerTransaction, 1));
        }
        if self.short() > Money::ZERO && state.player_organization() == Some(self.organization) {
            budget.push((IdKind::Report, 1));
        }
        budget
    }
}

impl PayrollOutcome {
    pub fn organization(&self) -> OrganizationId {
        self.organization
    }
    pub fn owed(&self) -> Money {
        self.owed
    }
    pub fn paid(&self) -> Money {
        self.paid
    }
    pub fn short(&self) -> Money {
        self.short
    }
    /// Canonical ledger transaction that paid this payroll. `None` means the organization
    /// had no spendable liquidity, so no money moved even though the wage obligation and
    /// shortfall consequences still resolved.
    pub fn transaction(&self) -> Option<LedgerTransactionId> {
        self.transaction
    }
}

/// Payroll shares the executive brief's day-boundary cadence: it runs exactly once per
/// simulated day and never at minute zero before any campaign time has passed.
fn is_payroll_due(now: SimTime) -> bool {
    crate::core::time::is_day_boundary(now)
}

/// Autonomous payroll pass over every active criminal organization, in stable organization-ID
/// order. A short treasury is distributed evenly across current organization members, to the cent, instead of
/// turning an almost-funded payroll into a total nonpayment. Financial mutation remains atomic.
///
/// Unexpected validation or allocation failures are propagated. Silently skipping an owed
/// payroll would forgive that day's wage obligation while still advancing campaign time, which
/// is neither retryable nor causally coherent.
pub(crate) fn apply_daily_payroll(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<PayrollOutcome>, PayrollError> {
    if !is_payroll_due(state.now()) {
        return Ok(Vec::new());
    }
    let organizations: Vec<OrganizationId> = state
        .world
        .organizations()
        .filter(|record| record.kind() == OrganizationKind::Criminal)
        .map(|record| record.id())
        .collect();
    let mut plans = Vec::with_capacity(organizations.len());
    for organization in organizations {
        if let Some(plan) = plan_organization_payroll(registry, state, organization)? {
            plans.push(plan);
        }
    }
    // Prove every non-ID finite dependency before the first organization mutates.
    // Account openings themselves are intentionally validated at commit because each organization
    // must observe the allocator position left by the preceding payroll; the aggregate ID budget
    // below guarantees those openings cannot exhaust.
    for plan in &plans {
        preflight_organization_payroll(registry, state, plan)?;
    }
    let mut budget = Vec::new();
    for plan in &plans {
        budget.extend(plan.id_budget(state));
    }
    if state.ids.reserve_many(&budget).is_err() {
        // The daily pass is one preflighted cohort. At the finite allocator rail there is no
        // representable persisted payday, so mutate none of the organizations rather than
        // paying an ID-ordered prefix or panicking the canonical tick after the day advanced.
        // Direct organization payroll transactions retain their typed exhaustion errors.
        return Ok(Vec::new());
    }

    let mut outcomes = Vec::with_capacity(plans.len());
    for plan in plans {
        outcomes.push(apply_organization_payroll(registry, state, plan)?);
    }
    Ok(outcomes)
}

fn plan_organization_payroll(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
) -> Result<Option<OrganizationPayrollPlan>, PayrollError> {
    // Payroll is a standing membership cost, not compensation sampled from the member's exact
    // availability at midnight. Custody blocks work while it lasts, but it does not end
    // membership or erase the day's wage obligation. Filtering on current detention here would
    // make a one-minute arrest spanning the boundary forgive a full day's wage while an all-day
    // detention ending one minute earlier would owe the full amount.
    let mut members: Vec<(CharacterId, Option<CharacterId>)> = state
        .world
        .characters_in_organization(organization)
        .map(|record| (record.id(), record.supervisor()))
        .collect();
    members.sort_unstable_by_key(|(member, _)| *member);
    if members.is_empty() {
        return Ok(None);
    }
    let per_member = registry.upkeep().per_member_daily();
    let owed = per_member
        .checked_mul(i64::try_from(members.len()).map_err(|_| PayrollError::MemberCountOverflow)?)
        .ok_or(PayrollError::ArithmeticOverflow)?;
    let funding = find_funding_accounts(state, organization);
    // Payroll is an organization-level obligation, so every organization-owned liquid cash
    // account is eligible. Enterprise records may reference the same cash account as one another
    // or as the general treasury; those references do not create ownership or segregation.
    let paid = resolve_payroll_liquidity(state, &funding, owed);
    let allocations = allocate_member_payments(
        &members,
        per_member,
        paid,
        payroll_remainder_offset(state.now(), members.len()),
    );
    Ok(Some(OrganizationPayrollPlan {
        organization,
        funding,
        per_member,
        owed,
        paid,
        allocations,
    }))
}

fn preflight_organization_payroll(
    registry: &Registry,
    state: &AppState,
    plan: &OrganizationPayrollPlan,
) -> Result<(), PayrollError> {
    use crate::core::version::ensure_version_can_advance;

    // Existing funding accounts that will actually be debited must retain one version slot.
    let mut remaining = plan.paid;
    for account_id in &plan.funding {
        if remaining == Money::ZERO {
            break;
        }
        let account = state
            .finance()
            .get_account(*account_id)
            .expect("planned payroll funding came from the finance owner index");
        let spendable = account.spendable_balance();
        if spendable == Money::ZERO {
            continue;
        }
        ensure_version_can_advance(account.version(), "financial account")
            .map_err(FinanceError::from)?;
        remaining = remaining
            .checked_sub(spendable.min(remaining))
            .expect("planned payroll debit cannot exceed remaining wages");
    }
    debug_assert_eq!(remaining, Money::ZERO);

    // Existing wage pockets also need one version slot and enough numeric balance headroom.
    // Missing pockets are covered by the aggregate FinancialAccount ID reservation.
    for (member, _, amount) in &plan.allocations {
        if *amount == Money::ZERO {
            continue;
        }
        if let Some(account) = find_existing_wage_account(state, *member, *amount) {
            ensure_version_can_advance(account.version(), "financial account")
                .map_err(FinanceError::from)?;
            account
                .balance()
                .checked_add(*amount)
                .ok_or(PayrollError::Finance(FinanceError::BalanceOverflow(
                    account.id(),
                )))?;
        }
    }

    let short = plan.short();
    if short > Money::ZERO {
        let outcome = PayrollOutcome {
            organization: plan.organization,
            owed: plan.owed,
            paid: plan.paid,
            short,
            transaction: None,
        };
        let underpaid: Vec<_> = plan
            .allocations
            .iter()
            .filter(|(_, _, amount)| *amount < plan.per_member)
            .copied()
            .collect();
        // This proves relationship version capacity and player-report shape. The resulting tokens
        // are discarded because each organization's real commit revalidates against the allocator
        // position produced by earlier disjoint payrolls.
        let _ = validate_shortfall_consequences(
            registry,
            state,
            plan.organization,
            &outcome,
            &underpaid,
            plan.per_member,
        )?;
    }
    Ok(())
}

fn apply_organization_payroll(
    registry: &Registry,
    state: &mut AppState,
    plan: OrganizationPayrollPlan,
) -> Result<PayrollOutcome, PayrollError> {
    let transaction = validate_payroll_payment(state, &plan.funding, &plan.allocations, plan.paid)?;
    let short = plan.short();
    let mut outcome = PayrollOutcome {
        organization: plan.organization,
        owed: plan.owed,
        paid: plan.paid,
        short,
        transaction: None,
    };
    let consequences = if short.cents() > 0 {
        let underpaid: Vec<_> = plan
            .allocations
            .iter()
            .filter(|(_, _, amount)| *amount < plan.per_member)
            .copied()
            .collect();
        Some(validate_shortfall_consequences(
            registry,
            state,
            plan.organization,
            &outcome,
            &underpaid,
            plan.per_member,
        )?)
    } else {
        None
    };

    // The shortfall report is the only consequence with a persistent ID. Reserve it before
    // the ledger or any relationship moves so a saturated report allocator cannot produce
    // paid wages and resentment without the player's causal report.
    if consequences
        .as_ref()
        .is_some_and(|plan| plan.report.is_some())
    {
        state.ids.reserve(IdKind::Report, 1)?;
    }
    if let Some(consequences) = &consequences {
        consequences.ensure_current(state)?;
    }
    if let Some(transaction) = transaction {
        outcome.transaction = Some(transaction.commit(state)?);
    }
    if let Some(consequences) = consequences {
        consequences.commit_preflighted(state);
    }
    Ok(outcome)
}

fn resolve_payroll_liquidity(
    state: &AppState,
    funding: &[FinancialAccountId],
    owed: Money,
) -> Money {
    let owed_cents = i128::from(owed.cents());
    let available_cents = funding
        .iter()
        .map(|account| {
            state
                .finance()
                .get_account(*account)
                .expect("payroll funding came from the finance owner index")
                .spendable_balance()
                .cents()
        })
        .fold(0_i128, |total, cents| {
            (total + i128::from(cents)).min(owed_cents)
        });
    Money::from_cents(
        i64::try_from(available_cents).expect("available payroll is bounded by money owed"),
    )
}

fn validate_payroll_payment(
    state: &AppState,
    funding: &[FinancialAccountId],
    allocations: &[(CharacterId, Option<CharacterId>, Money)],
    paid: Money,
) -> Result<Option<ValidatedLedgerTransaction>, PayrollError> {
    if paid == Money::ZERO {
        return Ok(None);
    }

    let mut postings = Vec::new();
    let mut remaining = paid;
    for account in funding {
        if remaining == Money::ZERO {
            break;
        }
        let spendable = state
            .finance()
            .get_account(*account)
            .expect("payroll funding came from the finance owner index")
            .spendable_balance();
        if spendable == Money::ZERO {
            continue;
        }
        let debit = spendable.min(remaining);
        postings.push(LedgerPosting {
            account: *account,
            amount: debit.checked_neg().expect("positive balance negates"),
        });
        remaining = remaining
            .checked_sub(debit)
            .expect("debit cannot exceed payable payroll");
    }
    debug_assert_eq!(remaining, Money::ZERO);

    let (wage_accounts, openings) = plan_wage_accounts(state, allocations)?;
    for ((_, _, amount), account) in allocations.iter().zip(wage_accounts) {
        if *amount == Money::ZERO {
            continue;
        }
        let account = account.expect("a positive wage allocation must resolve an account");
        postings.push(LedgerPosting {
            account,
            amount: *amount,
        });
    }
    debug_assert_eq!(
        postings
            .iter()
            .map(|posting| posting.amount.cents())
            .sum::<i64>(),
        0,
        "payroll postings must balance"
    );
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!("Daily payroll for {} member(s)", allocations.len()),
        postings,
        authorization: None,
    };
    let transaction = match openings {
        Some(openings) => validate_record_transaction_with_openings(state, openings, draft),
        None => validate_record_transaction(state, draft),
    }?;
    Ok(Some(transaction))
}

fn allocate_member_payments(
    members: &[(CharacterId, Option<CharacterId>)],
    per_member: Money,
    paid: Money,
    remainder_offset: usize,
) -> Vec<(CharacterId, Option<CharacterId>, Money)> {
    let count = i64::try_from(members.len()).expect("payroll member count must fit i64");
    let base = paid.cents() / count;
    let remainder = paid.cents() % count;
    members
        .iter()
        .enumerate()
        .map(|(index, (member, supervisor))| {
            // A stable CharacterId order makes allocation deterministic, but always giving the
            // low IDs the remainder cents turns creation order into a permanent wage advantage.
            // Rotate the remainder window once per payroll day so repeated tiny shortfalls are
            // shared fairly without adding mutable scheduling state.
            let relative = (index + members.len() - remainder_offset) % members.len();
            let extra = i64::from(
                i64::try_from(relative).expect("payroll member index must fit i64") < remainder,
            );
            let amount = Money::from_cents(base + extra).min(per_member);
            (*member, *supervisor, amount)
        })
        .collect()
}

/// First member eligible for a remainder cent on this payroll day. Day one begins at the
/// lowest stable member ID; later days advance one slot, so no persistent CharacterId ordering
/// advantage survives repeated sub-cent-per-member shortfalls. The rotation is keyed to the
/// current headcount, so joining or leaving members reset the fairness window rather than
/// inheriting rounding history from a different crew size.
fn payroll_remainder_offset(now: SimTime, member_count: usize) -> usize {
    debug_assert!(member_count > 0);
    debug_assert!(is_payroll_due(now));
    let payroll_day = now.as_minutes() / DAY_MINUTES;
    let zero_based_day = payroll_day
        .checked_sub(1)
        .expect("payroll is never due before the first completed simulation day");
    let member_count = u64::try_from(member_count)
        .expect("payroll member count must fit the simulation clock width");
    usize::try_from(zero_based_day % member_count)
        .expect("payroll remainder offset is bounded by the in-memory member count")
}

/// Resolves existing personal pockets and plans every missing pocket read-only. The returned
/// opening token is consumed by the same ledger transaction that first funds those accounts,
/// so a rejected payroll cannot consume account IDs or leave empty bookkeeping behind.
fn plan_wage_accounts(
    state: &AppState,
    allocations: &[(CharacterId, Option<CharacterId>, Money)],
) -> Result<
    (
        Vec<Option<FinancialAccountId>>,
        Option<ValidatedFinancialAccountOpenings>,
    ),
    FinanceError,
> {
    let mut resolved = vec![None; allocations.len()];
    let mut missing = Vec::new();
    let mut missing_positions = Vec::new();
    for (index, (member, _, amount)) in allocations.iter().enumerate() {
        if *amount == Money::ZERO {
            continue;
        }
        let owner = FinancialOwner::Character(*member);
        match find_existing_wage_account(state, *member, *amount).map(|account| account.id()) {
            Some(existing) => {
                resolved[index] = Some(existing);
            }
            None => {
                missing_positions.push(index);
                missing.push(FinancialAccountDraft {
                    owner,
                    kind: AccountKind::StreetCash,
                });
            }
        }
    }
    let openings = if missing.is_empty() {
        None
    } else {
        let openings = validate_open_accounts(state, missing)?;
        for (planned_index, position) in missing_positions.into_iter().enumerate() {
            resolved[position] = openings.account_id(planned_index);
        }
        Some(openings)
    };
    Ok((resolved, openings))
}

fn find_funding_accounts(
    state: &AppState,
    organization: OrganizationId,
) -> Vec<FinancialAccountId> {
    let owner = FinancialOwner::Organization(organization);
    let mut accounts: Vec<_> = state
        .finance()
        .accounts_for(owner)
        .filter_map(|account| {
            let spendable = account.spendable_balance();
            (spendable > Money::ZERO && payroll_account_has_posting_headroom(account.version()))
                .then_some((
                    account.kind().unrestricted_spending_priority(),
                    spendable,
                    account.id(),
                ))
        })
        .collect();
    // Wages are an informal carrying cost. Spend exposed street cash first, then hidden dirty
    // reserves, before consuming clean liquidity that can finance legitimate purchases and
    // delegated budgets. Within an equal semantic class, use the largest balance first to keep
    // ordinary payroll transactions compact; account ID is only the deterministic final tie.
    accounts.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.cmp(&right.2))
    });
    accounts.into_iter().map(|(_, _, id)| id).collect()
}

fn find_existing_wage_account(
    state: &AppState,
    member: CharacterId,
    amount: Money,
) -> Option<&crate::finance::FinancialAccountRecord> {
    state
        .finance()
        .accounts_for(FinancialOwner::Character(member))
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && payroll_account_has_posting_headroom(account.version())
                && account.balance().checked_add(amount).is_some()
        })
}

fn payroll_account_has_posting_headroom(version: u32) -> bool {
    version < u32::MAX
}

struct ValidatedPayrollShortfallConsequences {
    relationships: Vec<ValidatedRelationship>,
    report: Option<ValidatedReport>,
}

impl ValidatedPayrollShortfallConsequences {
    fn ensure_current(&self, state: &AppState) -> Result<(), RelationshipError> {
        for relationship in &self.relationships {
            relationship.ensure_current(state)?;
        }
        Ok(())
    }

    fn commit_preflighted(self, state: &mut AppState) {
        for relationship in self.relationships {
            relationship.commit_preflighted(state);
        }
        if let Some(report) = self.report {
            report
                .commit(state)
                .expect("payroll shortfall report ID was preflighted before mutation");
        }
    }
}

/// Pre-validates every shortfall consequence before payroll money moves. Resentment scales with
/// the uncovered share of each member's wage: a token rounding shortfall is noticeable but must
/// not cause the same relationship damage as receiving nothing. The player organization receives
/// a persisted notable report so the cause is discoverable.
fn validate_shortfall_consequences(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>, Money)],
    per_member: Money,
) -> Result<ValidatedPayrollShortfallConsequences, PayrollError> {
    let maximum_increment = registry.upkeep().shortfall_resentment();
    let mut relationships = Vec::new();
    for (member, supervisor, paid) in members {
        // Leadership absorbs its own shortfall: the boss has no supervisor to resent, as the
        // residual claimant of an underfunded organization. Only supervised members convert
        // an uncovered wage share into supervisor-directed resentment.
        let Some(supervisor) = supervisor else {
            continue;
        };
        let increment =
            resolve_shortfall_resentment_increment(maximum_increment, per_member, *paid);
        let current_dimensions = state
            .social()
            .get_relationship(*member, *supervisor)
            .map(|record| record.dimensions())
            .unwrap_or_else(RelationshipDimensions::zero);
        let mut dimensions = current_dimensions;
        dimensions.resentment = dimensions.resentment.saturating_add(increment);
        if dimensions == current_dimensions {
            continue;
        }
        match validate_set_relationship(state, *member, *supervisor, dimensions) {
            Ok(relationship) => relationships.push(relationship),
            // Payroll itself remains mandatory even when one social edge has consumed its final
            // representable revision. Direct relationship commands still report VersionCapacity;
            // the automatic shortfall pass simply cannot persist any further resentment on that
            // terminal edge and must not use it to block wages, other organizations, or reports.
            Err(RelationshipError::VersionCapacity(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let report = if state.player_organization() == Some(organization) {
        Some(validate_payroll_shortfall_report(
            state,
            organization,
            outcome,
            members,
        )?)
    } else {
        None
    };
    Ok(ValidatedPayrollShortfallConsequences {
        relationships,
        report,
    })
}

/// Scales the authored full-nonpayment resentment by the fraction of one wage left unpaid,
/// rounding any positive shortfall up to one point. The result is bounded by the authored
/// maximum and uses wide arithmetic so ordinary money values cannot overflow the ratio.
fn resolve_shortfall_resentment_increment(maximum: u8, owed: Money, paid: Money) -> u8 {
    debug_assert!(owed > Money::ZERO);
    debug_assert!(paid >= Money::ZERO && paid < owed);
    if maximum == 0 {
        return 0;
    }
    let owed_cents = i128::from(owed.cents());
    let short_cents = i128::from(
        owed.checked_sub(paid)
            .expect("underpaid payroll allocation cannot exceed its wage")
            .cents(),
    );
    let scaled = (i128::from(maximum) * short_cents + owed_cents - 1) / owed_cents;
    u8::try_from(scaled)
        .expect("scaled resentment is bounded by the authored u8 maximum")
        .min(maximum)
}

fn validate_payroll_shortfall_report(
    state: &AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>, Money)],
) -> Result<ValidatedReport, ReportError> {
    let mut entities = members
        .iter()
        .map(|(member, _, _)| EntityRef::Character(*member))
        .collect::<std::collections::BTreeSet<_>>();
    entities.insert(EntityRef::Organization(organization));
    validate_record_report(
        state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Financial,
            title: "Payroll ran short".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!(
                    "Payroll owed {} but only {} could be paid; {} went uncovered and the shorted crew noticed.",
                    format_money_cents(outcome.owed().cents()),
                    format_money_cents(outcome.paid().cents()),
                    format_money_cents(outcome.short().cents()),
                ),
                sources: Vec::new(),
                entities,
                decision: None,
            }],
        },
    )
}

#[cfg(test)]
mod tests;
