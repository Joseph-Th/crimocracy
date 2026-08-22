//! Explicit conversion of held operation property into ledger-backed liquid cash through a resale venue.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, FinancialAccountId, IdExhaustionError, IdKind, InformationId, LedgerTransactionId,
    OperationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::finance_system::{
    validate_record_transaction, FinanceError, ValidatedLedgerTransaction,
};
use crate::finance::{AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::{
    OperationCashDispositionRecord, OperationKind, OperationPropertyDispositionRecord,
    OperationStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{BusinessFunction, BusinessOwner, Lifecycle};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub struct PropertyDispositionDraft {
    pub operation: OperationId,
    pub venue: BusinessId,
    pub cash_account: FinancialAccountId,
    pub settlement_account: FinancialAccountId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyDispositionOutcome {
    pub transaction: LedgerTransactionId,
    pub information: InformationId,
    pub report: ReportId,
    pub realized_value: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PropertyDispositionError {
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} is not completed")]
    OperationNotCompleted(OperationId),
    #[error("operation {0} has no held property proceeds")]
    NoHeldProperty(OperationId),
    #[error("operation {0} has no held cash proceeds")]
    NoHeldCash(OperationId),
    #[error("operation {0} property has already been disposed")]
    AlreadyDisposed(OperationId),
    #[error("operation {0} cash has already been deposited")]
    AlreadyDeposited(OperationId),
    #[error("business {0} does not exist")]
    MissingVenue(BusinessId),
    #[error("venue {venue} neighborhood {neighborhood} does not exist")]
    MissingVenueNeighborhood {
        venue: BusinessId,
        neighborhood: crate::core::id::NeighborhoodId,
    },
    #[error("business {0} is not active")]
    InactiveVenue(BusinessId),
    #[error("business {0} does not provide resale-market access")]
    VenueNotResaleMarket(BusinessId),
    #[error("business {venue} is not owned by organization {organization}")]
    VenueOwnerMismatch {
        venue: BusinessId,
        organization: crate::core::id::OrganizationId,
    },
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("financial account {account} is not owned by organization {organization}")]
    AccountOwnerMismatch {
        account: FinancialAccountId,
        organization: crate::core::id::OrganizationId,
    },
    #[error("property liquidation cash account {0} must be street or concealed cash")]
    InvalidCashAccountKind(FinancialAccountId),
    #[error("property liquidation settlement account {0} must be a settlement account")]
    InvalidSettlementAccountKind(FinancialAccountId),
    #[error("property liquidation cash and settlement accounts must be distinct")]
    SameAccount,
    #[error("operation {0} property liquidation arithmetic overflowed")]
    ArithmeticOverflow(OperationId),
    #[error("operation {0} held property value is too small to liquidate")]
    NegligibleValue(OperationId),
    #[error("operation {operation} changed after disposition validation; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error("business {venue} changed after disposition validation; expected version {expected}, found {found}")]
    StaleVenue {
        venue: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "property disposition was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedPropertyDisposition {
    draft: PropertyDispositionDraft,
    expected_operation_version: u32,
    expected_venue_version: u32,
    disposed_at: SimTime,
    realized_value: Money,
    ledger: ValidatedLedgerTransaction,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedPropertyDisposition {
    pub fn realized_value(&self) -> Money {
        self.realized_value
    }

    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<PropertyDispositionOutcome, PropertyDispositionError> {
        state.ids.reserve_many(&[
            (IdKind::LedgerTransaction, 1),
            (IdKind::Information, 1),
            (IdKind::Report, 1),
        ])?;
        let operation = state.operations.get_operation(self.draft.operation).ok_or(
            PropertyDispositionError::MissingOperation(self.draft.operation),
        )?;
        if operation.version() != self.expected_operation_version {
            return Err(PropertyDispositionError::StaleOperation {
                operation: self.draft.operation,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        validate_operation_property_state(operation)?;
        let organization = operation.responsible_organization();
        let venue = state
            .world
            .get_business(self.draft.venue)
            .ok_or(PropertyDispositionError::MissingVenue(self.draft.venue))?;
        if venue.version() != self.expected_venue_version {
            return Err(PropertyDispositionError::StaleVenue {
                venue: self.draft.venue,
                expected: self.expected_venue_version,
                found: venue.version(),
            });
        }
        validate_venue(venue, organization)?;
        if state.now() != self.disposed_at {
            return Err(PropertyDispositionError::StaleTime {
                expected: self.disposed_at,
                found: state.now(),
            });
        }

        let transaction = self.ledger.commit(state)?;
        let information = self.information.commit(state)?;
        let report = self.report.commit(state)?;
        state.operations.set_property_disposition(
            self.draft.operation,
            OperationPropertyDispositionRecord {
                disposed_at: self.disposed_at,
                venue: self.draft.venue,
                venue_version: self.expected_venue_version,
                realized_value: self.realized_value,
                cash_account: self.draft.cash_account,
                settlement_account: self.draft.settlement_account,
                transaction,
                information,
                report,
            },
        );
        Ok(PropertyDispositionOutcome {
            transaction,
            information,
            report,
            realized_value: self.realized_value,
        })
    }
}

pub fn validate_dispose_property(
    registry: &Registry,
    state: &AppState,
    draft: PropertyDispositionDraft,
) -> Result<ValidatedPropertyDisposition, PropertyDispositionError> {
    let operation = state
        .operations
        .get_operation(draft.operation)
        .ok_or(PropertyDispositionError::MissingOperation(draft.operation))?;
    let proceeds = validate_operation_property_state(operation)?;
    let organization = operation.responsible_organization();
    let venue = state
        .world
        .get_business(draft.venue)
        .ok_or(PropertyDispositionError::MissingVenue(draft.venue))?;
    validate_venue(venue, organization)?;
    validate_accounts(
        state,
        organization,
        draft.cash_account,
        draft.settlement_account,
    )?;
    let realized_value = resolve_property_liquidation_value(
        registry,
        state,
        operation.kind(),
        proceeds.estimated_value(),
        draft.operation,
        draft.venue,
    )?;
    let negative_value = Money::from_cents(realized_value.cents().checked_neg().ok_or(
        PropertyDispositionError::ArithmeticOverflow(draft.operation),
    )?);
    let ledger = validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Property liquidation for {} through {}",
                draft.operation, draft.venue
            ),
            postings: vec![
                LedgerPosting {
                    account: draft.cash_account,
                    amount: realized_value,
                },
                LedgerPosting {
                    account: draft.settlement_account,
                    amount: negative_value,
                },
            ],
            authorization: None,
        },
    )?;
    let disposition_summary = build_disposition_summary(
        operation.title(),
        venue.name(),
        proceeds.estimated_value(),
        realized_value,
    );
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::Accountant,
            topic: InformationTopic::FinancialPerformance,
            source_entity: Some(EntityRef::Business(draft.venue)),
            subject: EntityRef::Operation(draft.operation),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: disposition_summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Financial,
            title: "Property disposition".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: disposition_summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Operation(draft.operation),
                    EntityRef::Business(draft.venue),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedPropertyDisposition {
        draft,
        expected_operation_version: operation.version(),
        expected_venue_version: venue.version(),
        disposed_at: state.now(),
        realized_value,
        ledger,
        information,
        report,
    })
}

fn validate_operation_property_state(
    operation: &crate::operations::OperationRecord,
) -> Result<crate::operations::OperationPropertyProceedsRecord, PropertyDispositionError> {
    if operation.status() != OperationStatus::Completed {
        return Err(PropertyDispositionError::OperationNotCompleted(
            operation.id(),
        ));
    }
    let proceeds = operation
        .resolution()
        .and_then(|resolution| resolution.property_proceeds())
        .ok_or(PropertyDispositionError::NoHeldProperty(operation.id()))?;
    if operation.property_disposition().is_some() {
        return Err(PropertyDispositionError::AlreadyDisposed(operation.id()));
    }
    Ok(proceeds)
}

fn validate_venue(
    venue: &crate::world::BusinessRecord,
    organization: crate::core::id::OrganizationId,
) -> Result<(), PropertyDispositionError> {
    if venue.lifecycle() != Lifecycle::Active {
        return Err(PropertyDispositionError::InactiveVenue(venue.id()));
    }
    if venue.owner() != BusinessOwner::Organization(organization) {
        return Err(PropertyDispositionError::VenueOwnerMismatch {
            venue: venue.id(),
            organization,
        });
    }
    if !venue.has_function(BusinessFunction::ResaleMarket) {
        return Err(PropertyDispositionError::VenueNotResaleMarket(venue.id()));
    }
    Ok(())
}

fn validate_accounts(
    state: &AppState,
    organization: crate::core::id::OrganizationId,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
) -> Result<(), PropertyDispositionError> {
    if cash_account == settlement_account {
        return Err(PropertyDispositionError::SameAccount);
    }
    let cash = state
        .finance
        .get_account(cash_account)
        .ok_or(PropertyDispositionError::MissingAccount(cash_account))?;
    let settlement = state
        .finance
        .get_account(settlement_account)
        .ok_or(PropertyDispositionError::MissingAccount(settlement_account))?;
    for account in [cash, settlement] {
        if account.owner() != FinancialOwner::Organization(organization) {
            return Err(PropertyDispositionError::AccountOwnerMismatch {
                account: account.id(),
                organization,
            });
        }
    }
    if !matches!(
        cash.kind(),
        AccountKind::StreetCash | AccountKind::ConcealedCash
    ) {
        return Err(PropertyDispositionError::InvalidCashAccountKind(
            cash_account,
        ));
    }
    if settlement.kind() != AccountKind::Settlement {
        return Err(PropertyDispositionError::InvalidSettlementAccountKind(
            settlement_account,
        ));
    }
    Ok(())
}

pub(crate) fn resolve_property_liquidation_value(
    registry: &Registry,
    state: &crate::core::state::AppState,
    kind: OperationKind,
    estimated_value: Money,
    operation: OperationId,
    venue: BusinessId,
) -> Result<Money, PropertyDispositionError> {
    let definition = registry
        .get_operation(kind)
        .execution()
        .property_proceeds()
        .ok_or(PropertyDispositionError::NoHeldProperty(operation))?;
    let mut recovery_basis = i32::from(definition.liquidation_recovery_basis_points());
    let venue_record = state
        .world
        .get_business(venue)
        .ok_or(PropertyDispositionError::MissingVenue(venue))?;
    let neighborhood = state
        .world
        .get_neighborhood(venue_record.neighborhood())
        .ok_or(PropertyDispositionError::MissingVenueNeighborhood {
            venue,
            neighborhood: venue_record.neighborhood(),
        })?;
    // Fencing conditions adjust the authored recovery rate: police scrutiny in the venue's
    // district suppresses recovery, quiet districts improve it. The clamp keeps the effective
    // rate inside the range the ledger and reporting arithmetic are validated for.
    let police = i32::from(neighborhood.profile().institutions.police_presence.value());
    let police_adjustment = (50 - police) * 20;
    recovery_basis = (recovery_basis + police_adjustment).clamp(3_000, 9_000);
    let value = i128::from(estimated_value.cents())
        .checked_mul(i128::from(recovery_basis))
        .ok_or(PropertyDispositionError::ArithmeticOverflow(operation))?
        / 10_000_i128;
    let cents = i64::try_from(value)
        .map_err(|_| PropertyDispositionError::ArithmeticOverflow(operation))?;
    if cents <= 0 {
        // Integer division legitimately rounds a tiny estimated value to zero; that is a
        // mundane small-haul case, not an overflow.
        return Err(PropertyDispositionError::NegligibleValue(operation));
    }
    Ok(Money::from_cents(cents))
}

pub(crate) fn build_disposition_summary(
    operation_title: &str,
    venue_name: &str,
    estimated_value: Money,
    realized_value: Money,
) -> String {
    format!(
        "Property from {operation_title} was liquidated through {venue_name} for {} cents from an estimated held value of {} cents.",
        realized_value.cents(),
        estimated_value.cents()
    )
}

#[derive(Clone, Copy, Debug)]
pub struct CashDispositionDraft {
    pub operation: OperationId,
    pub cash_account: FinancialAccountId,
    pub settlement_account: FinancialAccountId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CashDispositionOutcome {
    pub transaction: LedgerTransactionId,
    pub information: InformationId,
    pub report: ReportId,
    pub deposited_value: Money,
}

/// Moves a completed operation's held cash into an organization account. The settlement
/// account acts as the fictitious external counterparty, mirroring enterprise settlements:
/// the take comes from outside the ledger, so its credit is balanced against settlement.
pub fn validate_deposit_operation_cash(
    state: &AppState,
    draft: CashDispositionDraft,
) -> Result<ValidatedCashDisposition, PropertyDispositionError> {
    let operation = state
        .operations
        .get_operation(draft.operation)
        .ok_or(PropertyDispositionError::MissingOperation(draft.operation))?;
    let proceeds = validate_operation_cash_state(operation)?;
    let organization = operation.responsible_organization();
    validate_accounts(
        state,
        organization,
        draft.cash_account,
        draft.settlement_account,
    )?;
    let amount = proceeds.amount();
    let negative_value = Money::from_cents(amount.cents().checked_neg().ok_or(
        PropertyDispositionError::ArithmeticOverflow(draft.operation),
    )?);
    let ledger = validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!("Cash deposit for {}", draft.operation),
            postings: vec![
                LedgerPosting {
                    account: draft.cash_account,
                    amount,
                },
                LedgerPosting {
                    account: draft.settlement_account,
                    amount: negative_value,
                },
            ],
            authorization: None,
        },
    )?;
    let deposit_summary = build_deposit_summary(operation.title(), amount);
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::Accountant,
            topic: InformationTopic::FinancialPerformance,
            source_entity: Some(proceeds.target()),
            subject: EntityRef::Operation(draft.operation),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: deposit_summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Financial,
            title: "Cash deposit".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: deposit_summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Operation(draft.operation),
                    proceeds.target(),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedCashDisposition {
        draft,
        expected_operation_version: operation.version(),
        deposited_at: state.now(),
        deposited_value: amount,
        ledger,
        information,
        report,
    })
}

pub struct ValidatedCashDisposition {
    draft: CashDispositionDraft,
    expected_operation_version: u32,
    deposited_at: SimTime,
    deposited_value: Money,
    ledger: ValidatedLedgerTransaction,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedCashDisposition {
    pub fn deposited_value(&self) -> Money {
        self.deposited_value
    }

    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<CashDispositionOutcome, PropertyDispositionError> {
        state.ids.reserve_many(&[
            (IdKind::LedgerTransaction, 1),
            (IdKind::Information, 1),
            (IdKind::Report, 1),
        ])?;
        let operation = state.operations.get_operation(self.draft.operation).ok_or(
            PropertyDispositionError::MissingOperation(self.draft.operation),
        )?;
        if operation.version() != self.expected_operation_version {
            return Err(PropertyDispositionError::StaleOperation {
                operation: self.draft.operation,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        validate_operation_cash_state(operation)?;
        if state.now() != self.deposited_at {
            return Err(PropertyDispositionError::StaleTime {
                expected: self.deposited_at,
                found: state.now(),
            });
        }

        let transaction = self.ledger.commit(state)?;
        let information = self.information.commit(state)?;
        let report = self.report.commit(state)?;
        state.operations.set_cash_disposition(
            self.draft.operation,
            OperationCashDispositionRecord {
                disposed_at: self.deposited_at,
                realized_value: self.deposited_value,
                cash_account: self.draft.cash_account,
                settlement_account: self.draft.settlement_account,
                transaction,
                information,
                report,
            },
        );
        Ok(CashDispositionOutcome {
            transaction,
            information,
            report,
            deposited_value: self.deposited_value,
        })
    }
}

fn validate_operation_cash_state(
    operation: &crate::operations::OperationRecord,
) -> Result<crate::operations::OperationCashProceedsRecord, PropertyDispositionError> {
    if operation.status() != OperationStatus::Completed {
        return Err(PropertyDispositionError::OperationNotCompleted(
            operation.id(),
        ));
    }
    let proceeds = operation
        .resolution()
        .and_then(|resolution| resolution.cash_proceeds())
        .ok_or(PropertyDispositionError::NoHeldCash(operation.id()))?;
    if operation.cash_disposition().is_some() {
        return Err(PropertyDispositionError::AlreadyDeposited(operation.id()));
    }
    Ok(proceeds)
}

pub(crate) fn build_deposit_summary(operation_title: &str, amount: Money) -> String {
    format!(
        "Cash from {operation_title} was deposited for {} cents.",
        amount.cents()
    )
}
