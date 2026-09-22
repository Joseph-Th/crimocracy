//! Legal record vocabulary grouped by domain while preserving one `LegalState` owner.
//!
//! Child modules define record, draft, and derived-index shapes only. `LegalState` owns every
//! collection and synchronized mutation. This facade preserves the existing `crate::legal` API
//! and serialized field layout without keeping unrelated legal vocabularies in one monolith.

mod casework;
mod custody;
mod policing;
mod prosecution;
mod representation;

use serde::{Deserialize, Serialize};

pub use casework::{
    ALL_INVESTIGATION_WORK_KINDS, Admissibility, CaseWitnessDraft, CaseWitnessRecord,
    EvidenceDraft, EvidenceKind, EvidenceRecord, EvidenceReliability, EvidenceStrength,
    IncidentEvidenceDraft, IncidentIntakeDraft, IncidentWitnessDraft, InformantDisclosureDraft,
    InformantDisclosureRecord, InformantDraft, InformantRecord, InvestigationDraft,
    InvestigationRecord, InvestigationStatus, InvestigationWorkCancellation,
    InvestigationWorkCancellationReason, InvestigationWorkDraft, InvestigationWorkFactors,
    InvestigationWorkFocus, InvestigationWorkKind, InvestigationWorkOutcome,
    InvestigationWorkRecord, InvestigationWorkResolution, InvestigationWorkStatus,
    WitnessCooperation, WitnessStatementDraft, WitnessStatementRecord,
};
pub use custody::{ArrestDraft, ArrestRecord, ArrestStatus};
pub use policing::{
    DayMinute, DayMinuteError, JurisdictionDraft, JurisdictionRecord, PatrolDeploymentDraft,
    PatrolDeploymentRecord, PatrolDeploymentStatus, PatrolWindow, PatrolWindowError,
    PoliceResponsePatrolSnapshot, PoliceResponseRecord, PoliceResponseStatus,
};
pub use prosecution::{
    ProsecutionCaseDraft, ProsecutionCaseRecord, ProsecutionCaseStatus, ProsecutionReferralDraft,
    ProsecutionReferralRecord,
};
pub use representation::{
    LegalRepresentationDraft, LegalRepresentationEndReason, LegalRepresentationOrigin,
    LegalRepresentationRecord, LegalRepresentationStatus,
};

pub(crate) use casework::{
    EvidenceAssessment, EvidenceConnection, EvidenceIdentity, InvestigationWorkIdentity,
    InvestigationWorkRuntime,
};
pub(in crate::legal) use casework::{
    EvidenceIndexes, InformantIndexes, InvestigationIndexes, InvestigationWorkIndexes,
    WitnessIndexes,
};
pub(in crate::legal) use custody::ArrestIndexes;
pub(in crate::legal) use policing::{JurisdictionIndexes, PatrolIndexes, PoliceResponseIndexes};
pub(in crate::legal) use prosecution::ProsecutionIndexes;
pub(in crate::legal) use representation::LegalRepresentationIndexes;

pub(crate) use policing::{
    JurisdictionRevision, PatrolDeploymentRevision, PoliceResponseRouting, PoliceResponseState,
    PoliceResponseTiming,
};
pub(crate) use prosecution::{
    ProsecutionCaseContext, ProsecutionCaseLifecycle, ProsecutionCaseReferrals,
    ProsecutionCaseResolution, ProsecutionCaseResolutionArtifacts,
};
pub(crate) use representation::{
    LegalRepresentationArtifacts, LegalRepresentationLifecycle, LegalRepresentationParties,
    LegalRepresentationPayment,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct LegalIndexes {
    pub(in crate::legal) investigations: InvestigationIndexes,
    pub(in crate::legal) evidence: EvidenceIndexes,
    pub(in crate::legal) witnesses: WitnessIndexes,
    pub(in crate::legal) informants: InformantIndexes,
    pub(in crate::legal) work: InvestigationWorkIndexes,
    pub(in crate::legal) jurisdictions: JurisdictionIndexes,
    pub(in crate::legal) patrols: PatrolIndexes,
    pub(in crate::legal) police_responses: PoliceResponseIndexes,
    pub(in crate::legal) arrests: ArrestIndexes,
    pub(in crate::legal) representations: LegalRepresentationIndexes,
    pub(in crate::legal) prosecutions: ProsecutionIndexes,
}
