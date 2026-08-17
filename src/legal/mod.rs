//! Legal institutions, patrol deployment, investigations, evidence graphs, witnesses, and informants.
//!
//! `records.rs` owns the record, draft, enum, and index definitions; `legal_state.rs` owns
//! [`LegalState`], the single owner of every legal record and derived index. The eleven
//! subsystem files (`*_system.rs`, `case_graph.rs`, `investigation_work_execution.rs`)
//! implement validation, decision, and commit paths against those records. This facade
//! re-exports the exact public surface used by `crate::legal::*` consumers.

pub mod arrest_system;
pub mod case_graph;
pub mod informant_system;
pub mod investigation_system;
pub mod investigation_work_execution;
pub mod jurisdiction_system;
pub mod legal_representation_system;
pub mod patrol_system;
pub mod police_response_system;
pub mod prosecution_system;
pub mod witness_system;

mod records;
mod legal_state;

pub use records::{
    Admissibility, ALL_INVESTIGATION_WORK_KINDS, ArrestDraft, ArrestRecord, ArrestStatus,
    CaseWitnessDraft, CaseWitnessRecord, DayMinute, DayMinuteError, EvidenceDraft, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft, InformantDisclosureDraft, InformantDisclosureRecord, InformantDraft,
    InformantRecord, InformantStatus, InvestigationDraft, InvestigationRecord, InvestigationStatus,
    InvestigationWorkDraft, InvestigationWorkFactors, InvestigationWorkFocus, InvestigationWorkKind,
    InvestigationWorkOutcome, InvestigationWorkRecord, InvestigationWorkResolution,
    InvestigationWorkStatus, InvestigatorRole, JurisdictionDraft, JurisdictionRecord,
    LegalRepresentationDraft, LegalRepresentationRecord, LegalRepresentationStatus,
    LegalRepresentationEndReason, PatrolDeploymentDraft, PatrolDeploymentRecord,
    PatrolDeploymentStatus, PatrolWindow, PatrolWindowError, PoliceResponsePatrolSnapshot,
    PoliceResponseRecord, PoliceResponseStatus, ProsecutionCaseDraft, ProsecutionCaseRecord,
    ProsecutionCaseStatus, ProsecutionReferralDraft, ProsecutionReferralRecord,
    WitnessCooperation, WitnessStatementDraft, WitnessStatementRecord,
};
pub use legal_state::LegalState;
pub(crate) use records::ProsecutionCaseResolution;
pub(super) use records::{
    EvidenceAssessment, EvidenceConnection, EvidenceIdentity, InvestigationWorkIdentity,
    InvestigationWorkRuntime,
};
pub(crate) use records::{
    LegalRepresentationArtifacts, LegalRepresentationLifecycle, LegalRepresentationParties,
    LegalRepresentationPayment, PoliceResponseRouting, PoliceResponseState, PoliceResponseTiming,
    ProsecutionCaseContext, ProsecutionCaseLifecycle, ProsecutionCaseReferrals,
    ProsecutionCaseResolutionArtifacts,
};
