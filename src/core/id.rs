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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    fn take(counter: &mut u32, label: &'static str) -> u32 {
        let current = *counter;
        *counter = counter
            .checked_add(1)
            .unwrap_or_else(|| panic!("persistent {label} ID space exhausted"));
        current
    }

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

    pub(crate) fn next_organization(&mut self) -> OrganizationId {
        OrganizationId::from_raw(Self::take(&mut self.world.organization, "organization"))
    }

    pub(crate) fn next_character(&mut self) -> CharacterId {
        CharacterId::from_raw(Self::take(&mut self.world.character, "character"))
    }

    pub(crate) fn next_neighborhood(&mut self) -> NeighborhoodId {
        NeighborhoodId::from_raw(Self::take(&mut self.world.neighborhood, "neighborhood"))
    }

    pub(crate) fn next_business(&mut self) -> BusinessId {
        BusinessId::from_raw(Self::take(&mut self.world.business, "business"))
    }

    pub(crate) fn next_business_ownership_change(&mut self) -> BusinessOwnershipChangeId {
        BusinessOwnershipChangeId::from_raw(Self::take(
            &mut self.world.business_ownership_change,
            "business ownership change",
        ))
    }

    pub(crate) fn next_operation(&mut self) -> OperationId {
        OperationId::from_raw(Self::take(&mut self.operations.operation, "operation"))
    }

    pub(crate) fn next_opportunity(&mut self) -> OpportunityId {
        OpportunityId::from_raw(Self::take(&mut self.operations.opportunity, "opportunity"))
    }

    pub(crate) fn next_information(&mut self) -> InformationId {
        InformationId::from_raw(Self::take(&mut self.operations.information, "information"))
    }

    pub(crate) fn next_contact(&mut self) -> ContactId {
        ContactId::from_raw(Self::take(&mut self.operations.contact, "contact"))
    }

    pub(crate) fn next_contact_disclosure(&mut self) -> ContactDisclosureId {
        ContactDisclosureId::from_raw(Self::take(
            &mut self.operations.contact_disclosure,
            "contact disclosure",
        ))
    }

    pub(crate) fn next_investigation(&mut self) -> InvestigationId {
        InvestigationId::from_raw(Self::take(
            &mut self.legal.investigation.investigation,
            "investigation",
        ))
    }

    pub(crate) fn next_investigation_work(&mut self) -> InvestigationWorkId {
        InvestigationWorkId::from_raw(Self::take(
            &mut self.legal.investigation.investigation_work,
            "investigation work",
        ))
    }

    pub(crate) fn next_patrol_deployment(&mut self) -> PatrolDeploymentId {
        PatrolDeploymentId::from_raw(Self::take(
            &mut self.legal.investigation.patrol_deployment,
            "patrol deployment",
        ))
    }

    pub(crate) fn next_police_response(&mut self) -> PoliceResponseId {
        PoliceResponseId::from_raw(Self::take(
            &mut self.legal.investigation.police_response,
            "police response",
        ))
    }

    pub(crate) fn next_case_witness(&mut self) -> CaseWitnessId {
        CaseWitnessId::from_raw(Self::take(
            &mut self.legal.investigation.case_witness,
            "case witness",
        ))
    }

    pub(crate) fn next_witness_statement(&mut self) -> WitnessStatementId {
        WitnessStatementId::from_raw(Self::take(
            &mut self.legal.investigation.witness_statement,
            "witness statement",
        ))
    }

    pub(crate) fn next_informant(&mut self) -> InformantId {
        InformantId::from_raw(Self::take(
            &mut self.legal.investigation.informant,
            "informant",
        ))
    }

    pub(crate) fn next_informant_disclosure(&mut self) -> InformantDisclosureId {
        InformantDisclosureId::from_raw(Self::take(
            &mut self.legal.investigation.informant_disclosure,
            "informant disclosure",
        ))
    }

    pub(crate) fn next_evidence(&mut self) -> EvidenceId {
        EvidenceId::from_raw(Self::take(
            &mut self.legal.investigation.evidence,
            "evidence",
        ))
    }

    pub(crate) fn next_arrest(&mut self) -> ArrestId {
        ArrestId::from_raw(Self::take(&mut self.legal.investigation.arrest, "arrest"))
    }

    pub(crate) fn next_legal_representation(&mut self) -> LegalRepresentationId {
        LegalRepresentationId::from_raw(Self::take(
            &mut self.legal.proceedings.legal_representation,
            "legal representation",
        ))
    }

    pub(crate) fn next_prosecution_case(&mut self) -> ProsecutionCaseId {
        ProsecutionCaseId::from_raw(Self::take(
            &mut self.legal.proceedings.prosecution_case,
            "prosecution case",
        ))
    }

    pub(crate) fn next_prosecution_referral(&mut self) -> ProsecutionReferralId {
        ProsecutionReferralId::from_raw(Self::take(
            &mut self.legal.proceedings.prosecution_referral,
            "prosecution referral",
        ))
    }

    pub(crate) fn next_report(&mut self) -> ReportId {
        ReportId::from_raw(Self::take(&mut self.reporting_finance.report, "report"))
    }

    pub(crate) fn next_history_event(&mut self) -> HistoryEventId {
        HistoryEventId::from_raw(Self::take(
            &mut self.reporting_finance.history_event,
            "history event",
        ))
    }

    pub(crate) fn next_financial_account(&mut self) -> FinancialAccountId {
        FinancialAccountId::from_raw(Self::take(
            &mut self.reporting_finance.financial_account,
            "financial account",
        ))
    }

    pub(crate) fn next_ledger_transaction(&mut self) -> LedgerTransactionId {
        LedgerTransactionId::from_raw(Self::take(
            &mut self.reporting_finance.ledger_transaction,
            "ledger transaction",
        ))
    }

    pub(crate) fn next_decision_request(&mut self) -> DecisionRequestId {
        DecisionRequestId::from_raw(Self::take(
            &mut self.management.decision_request,
            "decision request",
        ))
    }

    pub(crate) fn next_mandate(&mut self) -> MandateId {
        MandateId::from_raw(Self::take(&mut self.management.mandate, "mandate"))
    }

    pub(crate) fn next_recruitment_attempt(&mut self) -> RecruitmentAttemptId {
        RecruitmentAttemptId::from_raw(Self::take(
            &mut self.management.recruitment_attempt,
            "recruitment attempt",
        ))
    }

    pub(crate) fn next_enterprise(&mut self) -> EnterpriseId {
        EnterpriseId::from_raw(Self::take(&mut self.economy.enterprise, "enterprise"))
    }

    pub(crate) fn next_enterprise_cycle(&mut self) -> EnterpriseCycleId {
        EnterpriseCycleId::from_raw(Self::take(
            &mut self.economy.enterprise_cycle,
            "enterprise cycle",
        ))
    }

    pub(crate) fn next_business_cycle(&mut self) -> BusinessCycleId {
        BusinessCycleId::from_raw(Self::take(
            &mut self.economy.business_cycle,
            "business cycle",
        ))
    }
}
