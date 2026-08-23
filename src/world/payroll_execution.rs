//! Daily organizational payroll: canonical wage charges, shortfall resentment, and reporting.
//!
//! Payroll is the organization's standing carrying cost. It runs at every campaign-day boundary
//! through the same production paths a player-driven payment would use: money moves only through
//! validated ledger transactions, relationship damage only through the canonical relationship
//! path, and player-visible consequences only through persisted reports. No random stream is
//! consumed, so payroll never perturbs any domain RNG sequence.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    helpers::format_money_cents, AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting,
    LedgerTransactionDraft, Money,
};
use crate::registry::Registry;
use crate::reports::report_system::validate_record_report;
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::RelationshipDimensions;
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

/// One organization's resolved payroll run for a single campaign day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayrollOutcome {
    organization: OrganizationId,
    owed: Money,
    paid: Money,
    short: Money,
    unpaid_members: Vec<CharacterId>,
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
    pub fn unpaid_members(&self) -> &[CharacterId] {
        &self.unpaid_members
    }
}

/// Payroll shares the executive brief's day-boundary cadence: it runs exactly once per
/// simulated day and never at minute zero before any campaign time has passed.
fn is_payroll_due(now: SimTime) -> bool {
    let minutes = now.as_minutes();
    minutes != 0 && minutes.is_multiple_of(1_440)
}

/// Autonomous payroll pass over every active criminal organization, in stable organization-ID
/// order. An organization whose payroll cannot fully validate keeps authoritative state unchanged
/// except for explicitly modeled diagnostics (the shortfall report), mirroring the other
/// autonomous passes in the tick pipeline.
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
        // An organization with no general cash economy at all has no payroll to run: its
        // members' standing is outside the modeled economy, and charging unpayable wages here
        // would manufacture resentment with no economic substrate behind it.
        let funding = funding_accounts(state, organization);
        if funding.is_empty() {
            continue;
        }
        if let Some(outcome) = apply_organization_payroll(registry, state, organization, &funding) {
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
) -> Option<PayrollOutcome> {
    let upkeep = registry.upkeep();
    // Detained members cannot work and are excluded from the wage bill, matching how every
    // other custody-facing system treats detention; wages resume on release.
    let members: Vec<(CharacterId, Option<CharacterId>)> = state
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
    if members.is_empty() {
        return None;
    }
    let per_member = upkeep.per_member_daily();
    let owed = per_member
        .checked_mul(i64::try_from(members.len()).ok()?)
        .expect("payroll total must not overflow money");

    // Wages are paid in full or not at all: a boss either meets payroll or the whole crew goes
    // unpaid and resentful. Funding drains the organization's general cash accounts only —
    // enterprise floats are delegated working capital under mandate authority — ordered by
    // balance then ID so the debit split is deterministic.
    let available: i64 = funding
        .iter()
        .filter_map(|account| state.finance().get_account(*account))
        .map(|record| record.balance().cents().max(0))
        .sum();
    let fully_funded = available >= owed.cents();
    let mut postings: Vec<LedgerPosting> = Vec::new();
    if fully_funded {
        let mut remaining = owed;
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
                .expect("debit cannot exceed owed");
        }
    }
    let paid = if fully_funded { owed } else { Money::ZERO };
    if paid.cents() > 0 {
        for member in members
            .iter()
            .map(|(member, _)| ensure_member_wage_account(state, *member))
        {
            postings.push(LedgerPosting {
                account: member,
                amount: per_member,
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
        let transaction = validate_record_transaction(
            state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: format!("Daily payroll for {} member(s)", members.len()),
                postings,
                authorization: None,
            },
        )
        .expect("fully funded payroll built from validated balances must validate");
        transaction
            .commit(state)
            .expect("validated payroll transaction must commit atomically");
    }

    let short = owed.checked_sub(paid).expect("paid cannot exceed owed");
    let outcome = PayrollOutcome {
        organization,
        owed,
        paid,
        short,
        unpaid_members: if short.cents() > 0 {
            members.iter().map(|(member, _)| *member).collect()
        } else {
            Vec::new()
        },
    };
    if short.cents() > 0 {
        apply_shortfall_consequences(registry, state, organization, &outcome, &members);
    }
    Some(outcome)
}

/// The member's personal street-cash pocket, created once on first pay and reused after;
/// wages land where later financial-satisfaction and bribery systems can find them.
fn ensure_member_wage_account(state: &mut AppState, member: CharacterId) -> FinancialAccountId {
    let owner = FinancialOwner::Character(member);
    if let Some(existing) = state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::StreetCash)
        .map(|account| account.id())
    {
        return existing;
    }
    insert_account(
        state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        },
    )
    .expect("a wage account for an existing member must validate")
}

fn funding_accounts(state: &AppState, organization: OrganizationId) -> Vec<FinancialAccountId> {
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

/// Unpaid work breeds resentment: each unpaid member's relationship toward their supervisor
/// gains the authored increment through the canonical relationship path, and the player
/// organization receives a persisted notable report so the cause is discoverable.
fn apply_shortfall_consequences(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>)],
) {
    let increment = registry.upkeep().shortfall_resentment();
    for (member, supervisor) in members {
        let Some(supervisor) = supervisor else {
            continue;
        };
        let mut dimensions = state
            .social()
            .get_relationship(*member, *supervisor)
            .map(|record| record.dimensions())
            .unwrap_or(zero_dimensions());
        let resentment = dimensions.resentment.value().saturating_add(increment);
        dimensions.resentment = crate::social::RelationshipLevel::try_new(resentment)
            .expect("clamped resentment must stay within the bounded range");
        validate_set_relationship(state, *member, *supervisor, dimensions)
            .expect("shortfall resentment between live members must validate")
            .commit(state);
    }
    if state.player_organization() == Some(organization) {
        report_payroll_shortfall(state, organization, outcome, members);
    }
}

fn report_payroll_shortfall(
    state: &mut AppState,
    organization: OrganizationId,
    outcome: &PayrollOutcome,
    members: &[(CharacterId, Option<CharacterId>)],
) {
    let mut entities = members
        .iter()
        .map(|(member, _)| EntityRef::Character(*member))
        .collect::<std::collections::BTreeSet<_>>();
    entities.insert(EntityRef::Organization(organization));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Financial,
            title: "Payroll ran short".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!(
                    "Payroll owed {} but only {} could be paid; {} went uncovered and the crew knows who went unpaid.",
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
    .expect("a payroll shortfall report about live entities must validate");
    report
        .commit(state)
        .expect("validated payroll shortfall report must commit");
}

fn zero_dimensions() -> RelationshipDimensions {
    let level = || crate::social::RelationshipLevel::try_new(0).expect("zero is a valid level");
    RelationshipDimensions {
        trust: level(),
        respect: level(),
        fear: level(),
        affection: level(),
        dependence: level(),
        resentment: level(),
        debt: level(),
    }
}

#[cfg(test)]
mod tests;
