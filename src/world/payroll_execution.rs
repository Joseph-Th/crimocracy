//! Daily organizational payroll: canonical wage charges, shortfall resentment, and reporting.
//!
//! Payroll is the organization's standing carrying cost. It runs at every campaign-day boundary
//! through the same production paths a player-driven payment would use: money moves only through
//! validated ledger transactions, relationship damage only through the canonical relationship
//! path, and player-visible consequences only through persisted reports. No random stream is
//! consumed, so payroll never perturbs any domain RNG sequence.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, FinancialAccountId, IdExhaustionError, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::finance_system::{
    FinanceError, ValidatedFinancialAccountOpenings, validate_open_accounts,
    validate_record_transaction, validate_record_transaction_with_openings,
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
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
enum PayrollError {
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
}

/// Payroll shares the executive brief's day-boundary cadence: it runs exactly once per
/// simulated day and never at minute zero before any campaign time has passed.
fn is_payroll_due(now: SimTime) -> bool {
    crate::core::time::is_day_boundary(now)
}

/// Autonomous payroll pass over every active criminal organization, in stable organization-ID
/// order. A short treasury is distributed evenly across active members, to the cent, instead of
/// turning an almost-funded payroll into a total nonpayment. Financial mutation remains atomic.
pub fn apply_daily_payroll(registry: &Registry, state: &mut AppState) -> Vec<PayrollOutcome> {
    if !is_payroll_due(state.now()) {
        return Vec::new();
    }
    let organizations: Vec<OrganizationId> = state
        .world
        .organizations()
        .filter(|record| record.kind() == OrganizationKind::Criminal)
        .map(|record| record.id())
        .collect();
    let mut outcomes = Vec::with_capacity(organizations.len());
    for organization in organizations {
        let outcome = match apply_organization_payroll(
            registry,
            state,
            organization,
            &find_funding_accounts(state, organization),
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                // Payroll is autonomous routine work. A ledger or allocation failure for one
                // organization must not crash the whole tick; the invariant suite will flag
                // the underlying state inconsistency, and payroll retries next day boundary.
                eprintln!("daily payroll for {organization} failed: {error}");
                continue;
            }
        };
        if let Some(outcome) = outcome {
            outcomes.push(outcome);
        }
    }
    outcomes
}

fn apply_organization_payroll(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    funding: &[FinancialAccountId],
) -> Result<Option<PayrollOutcome>, PayrollError> {
    let upkeep = registry.upkeep();
    // Detained members cannot work and are excluded from the wage bill, matching how every
    // other custody-facing system treats detention; wages resume on release.
    let mut members: Vec<(CharacterId, Option<CharacterId>)> = state
        .world
        .characters_in_organization(organization)
        .filter(|record| {
            state
                .legal
                .active_arrest_for_character(record.id())
                .is_none()
        })
        .map(|record| (record.id(), record.supervisor()))
        .collect();
    members.sort_unstable_by_key(|(member, _)| *member);
    if members.is_empty() {
        return Ok(None);
    }
    let per_member = upkeep.per_member_daily();
    let owed = per_member
        .checked_mul(i64::try_from(members.len()).map_err(|_| PayrollError::MemberCountOverflow)?)
        .ok_or(PayrollError::ArithmeticOverflow)?;

    // Funding drains the organization's general cash accounts only. Enterprise floats are
    // delegated working capital under mandate authority. We only need availability up to the
    // amount owed, so the i128 accumulator cannot overflow even if a campaign has many very
    // large positive accounts.
    let owed_cents = i128::from(owed.cents());
    let available_cents = funding
        .iter()
        .filter_map(|account| state.finance().get_account(*account))
        .map(|record| record.balance().cents().max(0))
        .fold(0_i128, |total, cents| {
            (total + i128::from(cents)).min(owed_cents)
        });
    let paid = Money::from_cents(
        i64::try_from(available_cents).expect("available payroll is bounded by money owed"),
    );
    let allocations = allocate_member_payments(&members, per_member, paid);
    let transaction = if paid > Money::ZERO {
        let mut postings: Vec<LedgerPosting> = Vec::new();
        let mut remaining = paid;
        for account in funding {
            if remaining.cents() == 0 {
                break;
            }
            let balance = state
                .finance()
                .get_account(*account)
                .map(|record| record.balance())
                .unwrap_or(Money::ZERO);
            if balance.cents() <= 0 {
                continue;
            }
            let debit = balance.min(remaining);
            postings.push(LedgerPosting {
                account: *account,
                amount: debit.checked_neg().expect("positive balance negates"),
            });
            remaining = remaining
                .checked_sub(debit)
                .expect("debit cannot exceed payable payroll");
        }
        let (wage_accounts, openings) = plan_wage_accounts(state, &allocations)?;
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
            memo: format!("Daily payroll for {} member(s)", members.len()),
            postings,
            authorization: None,
        };
        let transaction = match openings {
            Some(openings) => validate_record_transaction_with_openings(state, openings, draft),
            None => validate_record_transaction(state, draft),
        }?;
        Some(transaction)
    } else {
        None
    };

    let short = owed.checked_sub(paid).expect("paid cannot exceed owed");
    let outcome = PayrollOutcome {
        organization,
        owed,
        paid,
        short,
    };
    let consequences = if short.cents() > 0 {
        let underpaid: Vec<_> = allocations
            .iter()
            .filter(|(_, _, amount)| *amount < per_member)
            .map(|(member, supervisor, _)| (*member, *supervisor))
            .collect();
        Some(validate_shortfall_consequences(
            registry,
            state,
            organization,
            &outcome,
            &underpaid,
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
    if let Some(transaction) = transaction {
        transaction.commit(state)?;
    }
    if let Some(consequences) = consequences {
        consequences.commit(state);
    }
    Ok(Some(outcome))
}

fn allocate_member_payments(
    members: &[(CharacterId, Option<CharacterId>)],
    per_member: Money,
    paid: Money,
) -> Vec<(CharacterId, Option<CharacterId>, Money)> {
    let count = i64::try_from(members.len()).expect("payroll member count must fit i64");
    let base = paid.cents() / count;
    let remainder = paid.cents() % count;
    members
        .iter()
        .enumerate()
        .map(|(index, (member, supervisor))| {
            let extra = i64::from(
                i64::try_from(index).expect("payroll member index must fit i64") < remainder,
            );
            let amount = Money::from_cents(base + extra).min(per_member);
            (*member, *supervisor, amount)
        })
        .collect()
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
        match state
            .finance()
            .accounts_for(owner)
            .find(|account| account.kind() == AccountKind::StreetCash)
            .map(|account| account.id())
        {
            Some(existing) => {
                resolved[index] = Some(existing);
            }
            _ => {
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
    // Enterprise floats are delegated working capital governed by a manager's mandate; payroll
    // funds from the boss's general cash only. Raiding a book is an explicit governance act
    // (a mandate revision or ledger transfer), never an automatic wage drain.
    let mut enterprise_floats: BTreeSet<FinancialAccountId> = BTreeSet::new();
    for record in state
        .enterprises()
        .enterprises_for_organization(organization)
    {
        enterprise_floats.insert(record.cash_account());
    }
    let mut accounts: Vec<_> = state
        .finance()
        .accounts_for(owner)
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash
            )
        })
        .filter(|account| !enterprise_floats.contains(&account.id()))
        .map(|account| (account.balance(), account.id()))
        .collect();
    accounts.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    accounts.into_iter().map(|(_, id)| id).collect()
}

struct ValidatedPayrollShortfallConsequences {
    relationships: Vec<ValidatedRelationship>,
    report: Option<ValidatedReport>,
}

impl ValidatedPayrollShortfallConsequences {
    fn commit(self, state: &mut AppState) {
        for relationship in self.relationships {
            relationship.commit(state);
        }
        if let Some(report) = self.report {
            report
                .commit(state)
                .expect("payroll shortfall report ID was preflighted before mutation");
        }
    }
}

/// Pre-validates every shortfall consequence before payroll money moves. Underpaid work breeds
/// resentment toward each member's supervisor, and the player organization receives a persisted
/// notable report so the cause is discoverable.
fn validate_shortfall_consequences(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>)],
) -> Result<ValidatedPayrollShortfallConsequences, PayrollError> {
    let increment = registry.upkeep().shortfall_resentment();
    let mut relationships = Vec::new();
    for (member, supervisor) in members {
        let Some(supervisor) = supervisor else {
            continue;
        };
        let mut dimensions = state
            .social()
            .get_relationship(*member, *supervisor)
            .map(|record| record.dimensions())
            .unwrap_or_else(RelationshipDimensions::zero);
        dimensions.resentment = dimensions.resentment.saturating_add(increment);
        relationships.push(validate_set_relationship(
            state,
            *member,
            *supervisor,
            dimensions,
        )?);
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

fn validate_payroll_shortfall_report(
    state: &AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>)],
) -> Result<ValidatedReport, ReportError> {
    let mut entities = members
        .iter()
        .map(|(member, _)| EntityRef::Character(*member))
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
