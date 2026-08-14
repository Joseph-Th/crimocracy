//! Typed persistent identifiers and the state-owned deterministic ID allocator.

use serde::{Deserialize, Serialize};
use std::fmt;

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
define_id!(OperationId, "operation");
define_id!(OpportunityId, "opportunity");
define_id!(InformationId, "information");
define_id!(InvestigationId, "investigation");
define_id!(InvestigationWorkId, "investigation-work");
define_id!(CaseWitnessId, "case-witness");
define_id!(WitnessStatementId, "witness-statement");
define_id!(InformantId, "informant");
define_id!(InformantDisclosureId, "informant-disclosure");
define_id!(EvidenceId, "evidence");
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct IdCounters {
    organization: u32,
    character: u32,
    neighborhood: u32,
    business: u32,
    operation: u32,
    opportunity: u32,
    information: u32,
    investigation: u32,
    investigation_work: u32,
    case_witness: u32,
    witness_statement: u32,
    informant: u32,
    informant_disclosure: u32,
    evidence: u32,
    report: u32,
    history_event: u32,
    financial_account: u32,
    ledger_transaction: u32,
    decision_request: u32,
    mandate: u32,
    recruitment_attempt: u32,
    enterprise: u32,
    enterprise_cycle: u32,
    business_cycle: u32,
}

impl IdCounters {
    pub(crate) fn new() -> Self {
        Self {
            organization: 1,
            character: 1,
            neighborhood: 1,
            business: 1,
            operation: 1,
            opportunity: 1,
            information: 1,
            investigation: 1,
            investigation_work: 1,
            case_witness: 1,
            witness_statement: 1,
            informant: 1,
            informant_disclosure: 1,
            evidence: 1,
            report: 1,
            history_event: 1,
            financial_account: 1,
            ledger_transaction: 1,
            decision_request: 1,
            mandate: 1,
            recruitment_attempt: 1,
            enterprise: 1,
            enterprise_cycle: 1,
            business_cycle: 1,
        }
    }

    fn take(counter: &mut u32, label: &'static str) -> u32 {
        let current = *counter;
        *counter = counter
            .checked_add(1)
            .unwrap_or_else(|| panic!("persistent {label} ID space exhausted"));
        current
    }

    pub(crate) fn next_organization(&mut self) -> OrganizationId {
        OrganizationId::from_raw(Self::take(&mut self.organization, "organization"))
    }

    pub(crate) fn next_character(&mut self) -> CharacterId {
        CharacterId::from_raw(Self::take(&mut self.character, "character"))
    }

    pub(crate) fn next_neighborhood(&mut self) -> NeighborhoodId {
        NeighborhoodId::from_raw(Self::take(&mut self.neighborhood, "neighborhood"))
    }

    pub(crate) fn next_business(&mut self) -> BusinessId {
        BusinessId::from_raw(Self::take(&mut self.business, "business"))
    }

    pub(crate) fn next_operation(&mut self) -> OperationId {
        OperationId::from_raw(Self::take(&mut self.operation, "operation"))
    }

    pub(crate) fn next_opportunity(&mut self) -> OpportunityId {
        OpportunityId::from_raw(Self::take(&mut self.opportunity, "opportunity"))
    }

    pub(crate) fn next_information(&mut self) -> InformationId {
        InformationId::from_raw(Self::take(&mut self.information, "information"))
    }

    pub(crate) fn next_investigation(&mut self) -> InvestigationId {
        InvestigationId::from_raw(Self::take(&mut self.investigation, "investigation"))
    }

    pub(crate) fn next_investigation_work(&mut self) -> InvestigationWorkId {
        InvestigationWorkId::from_raw(Self::take(
            &mut self.investigation_work,
            "investigation work",
        ))
    }

    pub(crate) fn next_case_witness(&mut self) -> CaseWitnessId {
        CaseWitnessId::from_raw(Self::take(&mut self.case_witness, "case witness"))
    }

    pub(crate) fn next_witness_statement(&mut self) -> WitnessStatementId {
        WitnessStatementId::from_raw(Self::take(&mut self.witness_statement, "witness statement"))
    }

    pub(crate) fn next_informant(&mut self) -> InformantId {
        InformantId::from_raw(Self::take(&mut self.informant, "informant"))
    }

    pub(crate) fn next_informant_disclosure(&mut self) -> InformantDisclosureId {
        InformantDisclosureId::from_raw(Self::take(
            &mut self.informant_disclosure,
            "informant disclosure",
        ))
    }

    pub(crate) fn next_evidence(&mut self) -> EvidenceId {
        EvidenceId::from_raw(Self::take(&mut self.evidence, "evidence"))
    }

    pub(crate) fn next_report(&mut self) -> ReportId {
        ReportId::from_raw(Self::take(&mut self.report, "report"))
    }

    pub(crate) fn next_history_event(&mut self) -> HistoryEventId {
        HistoryEventId::from_raw(Self::take(&mut self.history_event, "history event"))
    }

    pub(crate) fn next_financial_account(&mut self) -> FinancialAccountId {
        FinancialAccountId::from_raw(Self::take(&mut self.financial_account, "financial account"))
    }

    pub(crate) fn next_ledger_transaction(&mut self) -> LedgerTransactionId {
        LedgerTransactionId::from_raw(Self::take(
            &mut self.ledger_transaction,
            "ledger transaction",
        ))
    }

    pub(crate) fn next_decision_request(&mut self) -> DecisionRequestId {
        DecisionRequestId::from_raw(Self::take(&mut self.decision_request, "decision request"))
    }

    pub(crate) fn next_mandate(&mut self) -> MandateId {
        MandateId::from_raw(Self::take(&mut self.mandate, "mandate"))
    }

    pub(crate) fn next_recruitment_attempt(&mut self) -> RecruitmentAttemptId {
        RecruitmentAttemptId::from_raw(Self::take(
            &mut self.recruitment_attempt,
            "recruitment attempt",
        ))
    }

    pub(crate) fn next_enterprise(&mut self) -> EnterpriseId {
        EnterpriseId::from_raw(Self::take(&mut self.enterprise, "enterprise"))
    }

    pub(crate) fn next_enterprise_cycle(&mut self) -> EnterpriseCycleId {
        EnterpriseCycleId::from_raw(Self::take(&mut self.enterprise_cycle, "enterprise cycle"))
    }

    pub(crate) fn next_business_cycle(&mut self) -> BusinessCycleId {
        BusinessCycleId::from_raw(Self::take(&mut self.business_cycle, "business cycle"))
    }
}
