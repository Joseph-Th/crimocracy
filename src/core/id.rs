//! Typed persistent identifiers and the state-owned deterministic ID allocator.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

macro_rules! define_id {
    ($name:ident, $label:literal) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(u32);

        impl $name {
            pub const fn raw(self) -> u32 {
                self.0
            }

            pub(crate) const fn from_raw(raw: u32) -> Self {
                Self(raw)
            }
        }

        impl PersistentId for $name {
            fn pid_raw(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($label, "-{}"), self.0)
            }
        }
    };
}

define_id!(OrganizationId, "org");
define_id!(CharacterId, "char");
define_id!(NeighborhoodId, "neighborhood");
define_id!(BusinessId, "business");
define_id!(BusinessOwnershipChangeId, "business-ownership");
define_id!(OperationId, "operation");
define_id!(OpportunityId, "opportunity");
define_id!(InformationId, "information");
define_id!(ContactId, "contact");
define_id!(ContactDisclosureId, "contact-disclosure");
define_id!(InvestigationId, "investigation");
define_id!(InvestigationWorkId, "investigation-work");
define_id!(PatrolDeploymentId, "patrol-deployment");
define_id!(PoliceResponseId, "police-response");
define_id!(CaseWitnessId, "case-witness");
define_id!(WitnessStatementId, "witness-statement");
define_id!(InformantId, "informant");
define_id!(InformantDisclosureId, "informant-disclosure");
define_id!(EvidenceId, "evidence");
define_id!(ArrestId, "arrest");
define_id!(LegalRepresentationId, "legal-representation");
define_id!(ProsecutionCaseId, "prosecution-case");
define_id!(ProsecutionReferralId, "prosecution-referral");
define_id!(ReportId, "report");
define_id!(HistoryEventId, "history");
define_id!(FinancialAccountId, "account");
define_id!(LedgerTransactionId, "transaction");
define_id!(DecisionRequestId, "decision");
define_id!(MandateId, "mandate");
define_id!(RecruitmentAttemptId, "recruitment");
define_id!(EnterpriseId, "enterprise");
define_id!(EnterpriseCycleId, "enterprise-cycle");
define_id!(BusinessCycleId, "business-cycle");

/// Raw-id extraction for id-keyed ordered maps; implemented by every [`define_id`] type.
pub(crate) trait PersistentId: Copy {
    fn pid_raw(self) -> u32;
}

/// `(smallest, largest)` raw key of a map keyed by a persistent id, read from the map's
/// key order in O(log n) per extreme. Allocator validation only needs these extremes:
/// ids are allocated monotonically, so any zero id would also be the smallest key.
pub(crate) trait IdKeyedBounds {
    fn id_bounds(&self) -> Option<(u32, u32)>;
}

impl<K, V> IdKeyedBounds for std::collections::BTreeMap<K, V>
where
    K: Ord + PersistentId,
{
    fn id_bounds(&self) -> Option<(u32, u32)> {
        let smallest = self.keys().next()?;
        Some((smallest.pid_raw(), self.keys().next_back()?.pid_raw()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorldIdCounters {
    organization: u32,
    character: u32,
    neighborhood: u32,
    business: u32,
    business_ownership_change: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationsIdCounters {
    operation: u32,
    opportunity: u32,
    information: u32,
    contact: u32,
    contact_disclosure: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InvestigationIdCounters {
    investigation: u32,
    investigation_work: u32,
    patrol_deployment: u32,
    police_response: u32,
    case_witness: u32,
    witness_statement: u32,
    informant: u32,
    informant_disclosure: u32,
    evidence: u32,
    arrest: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProceedingIdCounters {
    legal_representation: u32,
    prosecution_case: u32,
    prosecution_referral: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegalIdCounters {
    investigation: InvestigationIdCounters,
    proceedings: ProceedingIdCounters,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReportingFinanceIdCounters {
    report: u32,
    history_event: u32,
    financial_account: u32,
    ledger_transaction: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ManagementIdCounters {
    decision_request: u32,
    mandate: u32,
    recruitment_attempt: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EconomyIdCounters {
    enterprise: u32,
    enterprise_cycle: u32,
    business_cycle: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct IdCounters {
    world: WorldIdCounters,
    operations: OperationsIdCounters,
    legal: LegalIdCounters,
    reporting_finance: ReportingFinanceIdCounters,
    management: ManagementIdCounters,
    economy: EconomyIdCounters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum IdKind {
    Organization,
    Character,
    Neighborhood,
    Business,
    BusinessOwnershipChange,
    Operation,
    Opportunity,
    Information,
    Contact,
    ContactDisclosure,
    Investigation,
    InvestigationWork,
    PatrolDeployment,
    PoliceResponse,
    CaseWitness,
    WitnessStatement,
    Informant,
    InformantDisclosure,
    Evidence,
    Arrest,
    LegalRepresentation,
    ProsecutionCase,
    ProsecutionReferral,
    Report,
    HistoryEvent,
    FinancialAccount,
    LedgerTransaction,
    DecisionRequest,
    Mandate,
    RecruitmentAttempt,
    Enterprise,
    EnterpriseCycle,
    BusinessCycle,
}

impl IdKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Character => "character",
            Self::Neighborhood => "neighborhood",
            Self::Business => "business",
            Self::BusinessOwnershipChange => "business ownership change",
            Self::Operation => "operation",
            Self::Opportunity => "opportunity",
            Self::Information => "information",
            Self::Contact => "contact",
            Self::ContactDisclosure => "contact disclosure",
            Self::Investigation => "investigation",
            Self::InvestigationWork => "investigation work",
            Self::PatrolDeployment => "patrol deployment",
            Self::PoliceResponse => "police response",
            Self::CaseWitness => "case witness",
            Self::WitnessStatement => "witness statement",
            Self::Informant => "informant",
            Self::InformantDisclosure => "informant disclosure",
            Self::Evidence => "evidence",
            Self::Arrest => "arrest",
            Self::LegalRepresentation => "legal representation",
            Self::ProsecutionCase => "prosecution case",
            Self::ProsecutionReferral => "prosecution referral",
            Self::Report => "report",
            Self::HistoryEvent => "history event",
            Self::FinancialAccount => "financial account",
            Self::LedgerTransaction => "ledger transaction",
            Self::DecisionRequest => "decision request",
            Self::Mandate => "mandate",
            Self::RecruitmentAttempt => "recruitment attempt",
            Self::Enterprise => "enterprise",
            Self::EnterpriseCycle => "enterprise cycle",
            Self::BusinessCycle => "business cycle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum IdExhaustionError {
    #[error("persistent {kind} ID space is exhausted (next value {next})")]
    Exhausted { kind: &'static str, next: u32 },
}

impl IdCounters {
    pub(crate) fn new() -> Self {
        Self {
            world: WorldIdCounters {
                organization: 1,
                character: 1,
                neighborhood: 1,
                business: 1,
                business_ownership_change: 1,
            },
            operations: OperationsIdCounters {
                operation: 1,
                opportunity: 1,
                information: 1,
                contact: 1,
                contact_disclosure: 1,
            },
            legal: LegalIdCounters {
                investigation: InvestigationIdCounters {
                    investigation: 1,
                    investigation_work: 1,
                    patrol_deployment: 1,
                    police_response: 1,
                    case_witness: 1,
                    witness_statement: 1,
                    informant: 1,
                    informant_disclosure: 1,
                    evidence: 1,
                    arrest: 1,
                },
                proceedings: ProceedingIdCounters {
                    legal_representation: 1,
                    prosecution_case: 1,
                    prosecution_referral: 1,
                },
            },
            reporting_finance: ReportingFinanceIdCounters {
                report: 1,
                history_event: 1,
                financial_account: 1,
                ledger_transaction: 1,
            },
            management: ManagementIdCounters {
                decision_request: 1,
                mandate: 1,
                recruitment_attempt: 1,
            },
            economy: EconomyIdCounters {
                enterprise: 1,
                enterprise_cycle: 1,
                business_cycle: 1,
            },
        }
    }

    /// Reads the effective next value for a kind (no mutation).
    pub(crate) const fn next_raw(&self, kind: IdKind) -> u32 {
        match kind {
            IdKind::Organization => self.world.organization,
            IdKind::Character => self.world.character,
            IdKind::Neighborhood => self.world.neighborhood,
            IdKind::Business => self.world.business,
            IdKind::BusinessOwnershipChange => self.world.business_ownership_change,
            IdKind::Operation => self.operations.operation,
            IdKind::Opportunity => self.operations.opportunity,
            IdKind::Information => self.operations.information,
            IdKind::Contact => self.operations.contact,
            IdKind::ContactDisclosure => self.operations.contact_disclosure,
            IdKind::Investigation => self.legal.investigation.investigation,
            IdKind::InvestigationWork => self.legal.investigation.investigation_work,
            IdKind::PatrolDeployment => self.legal.investigation.patrol_deployment,
            IdKind::PoliceResponse => self.legal.investigation.police_response,
            IdKind::CaseWitness => self.legal.investigation.case_witness,
            IdKind::WitnessStatement => self.legal.investigation.witness_statement,
            IdKind::Informant => self.legal.investigation.informant,
            IdKind::InformantDisclosure => self.legal.investigation.informant_disclosure,
            IdKind::Evidence => self.legal.investigation.evidence,
            IdKind::Arrest => self.legal.investigation.arrest,
            IdKind::LegalRepresentation => self.legal.proceedings.legal_representation,
            IdKind::ProsecutionCase => self.legal.proceedings.prosecution_case,
            IdKind::ProsecutionReferral => self.legal.proceedings.prosecution_referral,
            IdKind::Report => self.reporting_finance.report,
            IdKind::HistoryEvent => self.reporting_finance.history_event,
            IdKind::FinancialAccount => self.reporting_finance.financial_account,
            IdKind::LedgerTransaction => self.reporting_finance.ledger_transaction,
            IdKind::DecisionRequest => self.management.decision_request,
            IdKind::Mandate => self.management.mandate,
            IdKind::RecruitmentAttempt => self.management.recruitment_attempt,
            IdKind::Enterprise => self.economy.enterprise,
            IdKind::EnterpriseCycle => self.economy.enterprise_cycle,
            IdKind::BusinessCycle => self.economy.business_cycle,
        }
    }

    /// Pre-flight availability check for allocating `count` more IDs of `kind` without mutating
    /// anything. Every composite commit reserves its full ID budget up front so that no later
    /// allocation can exhaust and strand an already-mutated owner. A counter of `u32::MAX` cannot
    /// allocate any further ID (incrementing it would overflow), so the last representable
    /// allocation is when the counter reads `u32::MAX - 1`.
    pub(crate) fn reserve(&self, kind: IdKind, count: u32) -> Result<(), IdExhaustionError> {
        let next = self.next_raw(kind);
        debug_assert!(next >= 1, "ID counters start at 1 and only increment");
        if next.checked_add(count).is_none() {
            return Err(IdExhaustionError::Exhausted {
                kind: kind.label(),
                next,
            });
        }
        Ok(())
    }

    /// Pre-flight availability check for a whole budget of kinds resolved as one atomic unit.
    pub(crate) fn reserve_many(&self, budget: &[(IdKind, u32)]) -> Result<(), IdExhaustionError> {
        let mut aggregated: std::collections::BTreeMap<IdKind, u32> =
            std::collections::BTreeMap::new();
        for (kind, count) in budget {
            let entry = aggregated.entry(*kind).or_insert(0);
            *entry = entry
                .checked_add(*count)
                .ok_or(IdExhaustionError::Exhausted {
                    kind: kind.label(),
                    next: self.next_raw(*kind),
                })?;
        }
        for (kind, total) in aggregated {
            self.reserve(kind, total)?;
        }
        Ok(())
    }

    fn take(counter: &mut u32, label: &'static str) -> Result<u32, IdExhaustionError> {
        let current = *counter;
        let Some(next) = counter.checked_add(1) else {
            return Err(IdExhaustionError::Exhausted {
                kind: label,
                next: current,
            });
        };
        *counter = next;
        Ok(current)
    }

    #[cfg(test)]
    pub(crate) fn set_next_raw_for_test(&mut self, kind: IdKind, next: u32) {
        match kind {
            IdKind::Organization => self.world.organization = next,
            IdKind::Character => self.world.character = next,
            IdKind::Neighborhood => self.world.neighborhood = next,
            IdKind::Business => self.world.business = next,
            IdKind::BusinessOwnershipChange => self.world.business_ownership_change = next,
            IdKind::Operation => self.operations.operation = next,
            IdKind::Opportunity => self.operations.opportunity = next,
            IdKind::Information => self.operations.information = next,
            IdKind::Contact => self.operations.contact = next,
            IdKind::ContactDisclosure => self.operations.contact_disclosure = next,
            IdKind::Investigation => self.legal.investigation.investigation = next,
            IdKind::InvestigationWork => self.legal.investigation.investigation_work = next,
            IdKind::PatrolDeployment => self.legal.investigation.patrol_deployment = next,
            IdKind::PoliceResponse => self.legal.investigation.police_response = next,
            IdKind::CaseWitness => self.legal.investigation.case_witness = next,
            IdKind::WitnessStatement => self.legal.investigation.witness_statement = next,
            IdKind::Informant => self.legal.investigation.informant = next,
            IdKind::InformantDisclosure => self.legal.investigation.informant_disclosure = next,
            IdKind::Evidence => self.legal.investigation.evidence = next,
            IdKind::Arrest => self.legal.investigation.arrest = next,
            IdKind::LegalRepresentation => self.legal.proceedings.legal_representation = next,
            IdKind::ProsecutionCase => self.legal.proceedings.prosecution_case = next,
            IdKind::ProsecutionReferral => self.legal.proceedings.prosecution_referral = next,
            IdKind::Report => self.reporting_finance.report = next,
            IdKind::HistoryEvent => self.reporting_finance.history_event = next,
            IdKind::FinancialAccount => self.reporting_finance.financial_account = next,
            IdKind::LedgerTransaction => self.reporting_finance.ledger_transaction = next,
            IdKind::DecisionRequest => self.management.decision_request = next,
            IdKind::Mandate => self.management.mandate = next,
            IdKind::RecruitmentAttempt => self.management.recruitment_attempt = next,
            IdKind::Enterprise => self.economy.enterprise = next,
            IdKind::EnterpriseCycle => self.economy.enterprise_cycle = next,
            IdKind::BusinessCycle => self.economy.business_cycle = next,
        }
    }

    pub(crate) fn next_organization(&mut self) -> Result<OrganizationId, IdExhaustionError> {
        Ok(OrganizationId::from_raw(Self::take(
            &mut self.world.organization,
            "organization",
        )?))
    }

    pub(crate) fn next_character(&mut self) -> Result<CharacterId, IdExhaustionError> {
        Ok(CharacterId::from_raw(Self::take(
            &mut self.world.character,
            "character",
        )?))
    }

    pub(crate) fn next_neighborhood(&mut self) -> Result<NeighborhoodId, IdExhaustionError> {
        Ok(NeighborhoodId::from_raw(Self::take(
            &mut self.world.neighborhood,
            "neighborhood",
        )?))
    }

    pub(crate) fn next_business(&mut self) -> Result<BusinessId, IdExhaustionError> {
        Ok(BusinessId::from_raw(Self::take(
            &mut self.world.business,
            "business",
        )?))
    }

    pub(crate) fn next_business_ownership_change(
        &mut self,
    ) -> Result<BusinessOwnershipChangeId, IdExhaustionError> {
        Ok(BusinessOwnershipChangeId::from_raw(Self::take(
            &mut self.world.business_ownership_change,
            "business ownership change",
        )?))
    }

    pub(crate) fn next_operation(&mut self) -> Result<OperationId, IdExhaustionError> {
        Ok(OperationId::from_raw(Self::take(
            &mut self.operations.operation,
            "operation",
        )?))
    }

    pub(crate) fn next_opportunity(&mut self) -> Result<OpportunityId, IdExhaustionError> {
        Ok(OpportunityId::from_raw(Self::take(
            &mut self.operations.opportunity,
            "opportunity",
        )?))
    }

    pub(crate) fn next_information(&mut self) -> Result<InformationId, IdExhaustionError> {
        Ok(InformationId::from_raw(Self::take(
            &mut self.operations.information,
            "information",
        )?))
    }

    pub(crate) fn next_contact(&mut self) -> Result<ContactId, IdExhaustionError> {
        Ok(ContactId::from_raw(Self::take(
            &mut self.operations.contact,
            "contact",
        )?))
    }

    pub(crate) fn next_contact_disclosure(
        &mut self,
    ) -> Result<ContactDisclosureId, IdExhaustionError> {
        Ok(ContactDisclosureId::from_raw(Self::take(
            &mut self.operations.contact_disclosure,
            "contact disclosure",
        )?))
    }

    pub(crate) fn next_investigation(&mut self) -> Result<InvestigationId, IdExhaustionError> {
        Ok(InvestigationId::from_raw(Self::take(
            &mut self.legal.investigation.investigation,
            "investigation",
        )?))
    }

    pub(crate) fn next_investigation_work(
        &mut self,
    ) -> Result<InvestigationWorkId, IdExhaustionError> {
        Ok(InvestigationWorkId::from_raw(Self::take(
            &mut self.legal.investigation.investigation_work,
            "investigation work",
        )?))
    }

    pub(crate) fn next_patrol_deployment(
        &mut self,
    ) -> Result<PatrolDeploymentId, IdExhaustionError> {
        Ok(PatrolDeploymentId::from_raw(Self::take(
            &mut self.legal.investigation.patrol_deployment,
            "patrol deployment",
        )?))
    }

    pub(crate) fn next_police_response(&mut self) -> Result<PoliceResponseId, IdExhaustionError> {
        Ok(PoliceResponseId::from_raw(Self::take(
            &mut self.legal.investigation.police_response,
            "police response",
        )?))
    }

    pub(crate) fn next_case_witness(&mut self) -> Result<CaseWitnessId, IdExhaustionError> {
        Ok(CaseWitnessId::from_raw(Self::take(
            &mut self.legal.investigation.case_witness,
            "case witness",
        )?))
    }

    pub(crate) fn next_witness_statement(
        &mut self,
    ) -> Result<WitnessStatementId, IdExhaustionError> {
        Ok(WitnessStatementId::from_raw(Self::take(
            &mut self.legal.investigation.witness_statement,
            "witness statement",
        )?))
    }

    pub(crate) fn next_informant(&mut self) -> Result<InformantId, IdExhaustionError> {
        Ok(InformantId::from_raw(Self::take(
            &mut self.legal.investigation.informant,
            "informant",
        )?))
    }

    pub(crate) fn next_informant_disclosure(
        &mut self,
    ) -> Result<InformantDisclosureId, IdExhaustionError> {
        Ok(InformantDisclosureId::from_raw(Self::take(
            &mut self.legal.investigation.informant_disclosure,
            "informant disclosure",
        )?))
    }

    pub(crate) fn next_evidence(&mut self) -> Result<EvidenceId, IdExhaustionError> {
        Ok(EvidenceId::from_raw(Self::take(
            &mut self.legal.investigation.evidence,
            "evidence",
        )?))
    }

    pub(crate) fn next_arrest(&mut self) -> Result<ArrestId, IdExhaustionError> {
        Ok(ArrestId::from_raw(Self::take(
            &mut self.legal.investigation.arrest,
            "arrest",
        )?))
    }

    pub(crate) fn next_legal_representation(
        &mut self,
    ) -> Result<LegalRepresentationId, IdExhaustionError> {
        Ok(LegalRepresentationId::from_raw(Self::take(
            &mut self.legal.proceedings.legal_representation,
            "legal representation",
        )?))
    }

    pub(crate) fn next_prosecution_case(&mut self) -> Result<ProsecutionCaseId, IdExhaustionError> {
        Ok(ProsecutionCaseId::from_raw(Self::take(
            &mut self.legal.proceedings.prosecution_case,
            "prosecution case",
        )?))
    }

    pub(crate) fn next_prosecution_referral(
        &mut self,
    ) -> Result<ProsecutionReferralId, IdExhaustionError> {
        Ok(ProsecutionReferralId::from_raw(Self::take(
            &mut self.legal.proceedings.prosecution_referral,
            "prosecution referral",
        )?))
    }

    pub(crate) fn next_report(&mut self) -> Result<ReportId, IdExhaustionError> {
        Ok(ReportId::from_raw(Self::take(
            &mut self.reporting_finance.report,
            "report",
        )?))
    }

    pub(crate) fn next_history_event(&mut self) -> Result<HistoryEventId, IdExhaustionError> {
        Ok(HistoryEventId::from_raw(Self::take(
            &mut self.reporting_finance.history_event,
            "history event",
        )?))
    }

    pub(crate) fn next_financial_account(
        &mut self,
    ) -> Result<FinancialAccountId, IdExhaustionError> {
        Ok(FinancialAccountId::from_raw(Self::take(
            &mut self.reporting_finance.financial_account,
            "financial account",
        )?))
    }

    pub(crate) fn next_ledger_transaction(
        &mut self,
    ) -> Result<LedgerTransactionId, IdExhaustionError> {
        Ok(LedgerTransactionId::from_raw(Self::take(
            &mut self.reporting_finance.ledger_transaction,
            "ledger transaction",
        )?))
    }

    pub(crate) fn next_decision_request(&mut self) -> Result<DecisionRequestId, IdExhaustionError> {
        Ok(DecisionRequestId::from_raw(Self::take(
            &mut self.management.decision_request,
            "decision request",
        )?))
    }

    pub(crate) fn next_mandate(&mut self) -> Result<MandateId, IdExhaustionError> {
        Ok(MandateId::from_raw(Self::take(
            &mut self.management.mandate,
            "mandate",
        )?))
    }

    pub(crate) fn next_recruitment_attempt(
        &mut self,
    ) -> Result<RecruitmentAttemptId, IdExhaustionError> {
        Ok(RecruitmentAttemptId::from_raw(Self::take(
            &mut self.management.recruitment_attempt,
            "recruitment attempt",
        )?))
    }

    pub(crate) fn next_enterprise(&mut self) -> Result<EnterpriseId, IdExhaustionError> {
        Ok(EnterpriseId::from_raw(Self::take(
            &mut self.economy.enterprise,
            "enterprise",
        )?))
    }

    pub(crate) fn next_enterprise_cycle(&mut self) -> Result<EnterpriseCycleId, IdExhaustionError> {
        Ok(EnterpriseCycleId::from_raw(Self::take(
            &mut self.economy.enterprise_cycle,
            "enterprise cycle",
        )?))
    }

    pub(crate) fn next_business_cycle(&mut self) -> Result<BusinessCycleId, IdExhaustionError> {
        Ok(BusinessCycleId::from_raw(Self::take(
            &mut self.economy.business_cycle,
            "business cycle",
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_at_u32_max_is_a_typed_recoverable_error() {
        let mut counters = IdCounters::new();
        counters.set_next_raw_for_test(IdKind::Operation, u32::MAX);
        let error = counters
            .next_operation()
            .expect_err("operation counter at u32::MAX cannot allocate another ID");
        assert!(matches!(
            error,
            IdExhaustionError::Exhausted {
                kind: "operation",
                next: u32::MAX
            }
        ));
    }

    #[test]
    fn last_representable_allocation_is_u32_max_minus_one() {
        let mut counters = IdCounters::new();
        counters.set_next_raw_for_test(IdKind::Character, u32::MAX - 1);
        let last = counters
            .next_character()
            .expect("counter at u32::MAX-1 may still allocate that exact raw value");
        assert_eq!(last.raw(), u32::MAX - 1);
        // The counter advanced to u32::MAX; the following allocation is the exhaustion point.
        counters
            .next_character()
            .expect_err("counter at u32::MAX cannot allocate");
    }

    #[test]
    fn reserve_checks_availability_without_mutating() {
        let mut counters = IdCounters::new();
        counters.set_next_raw_for_test(IdKind::Business, u32::MAX - 1);
        // One more fits; two do not.
        assert!(counters.reserve(IdKind::Business, 1).is_ok());
        assert!(matches!(
            counters.reserve(IdKind::Business, 2),
            Err(IdExhaustionError::Exhausted { .. })
        ));
        // Reserve of zero is always fine and never mutates the counter.
        let before = counters.next_raw(IdKind::Business);
        assert!(counters.reserve(IdKind::Business, 0).is_ok());
        assert_eq!(counters.next_raw(IdKind::Business), before);
        // Exhaustion resets nothing: the counter still reads u32::MAX.
        counters.set_next_raw_for_test(IdKind::Business, u32::MAX);
        counters
            .reserve(IdKind::Business, 1)
            .expect_err("counter at u32::MAX cannot reserve one more");
        assert_eq!(counters.next_raw(IdKind::Business), u32::MAX);
    }

    #[test]
    fn reserve_many_reports_the_first_exhausted_kind() {
        let mut counters = IdCounters::new();
        counters.set_next_raw_for_test(IdKind::Information, u32::MAX);
        counters.set_next_raw_for_test(IdKind::Report, 5);
        let budget = [
            (IdKind::Report, 1),
            (IdKind::Information, 1),
            (IdKind::HistoryEvent, 1),
        ];
        let error = counters
            .reserve_many(&budget)
            .expect_err("exhausted information must abort the combined reservation");
        assert!(matches!(
            error,
            IdExhaustionError::Exhausted {
                kind: "information",
                ..
            }
        ));
    }
}
