//! Legal-representation records, lifecycle artifacts, drafts, and live indexes.

use crate::core::id::{
    ArrestId, CharacterId, ContactId, FinancialAccountId, InformationId, LedgerTransactionId,
    LegalRepresentationId, OrganizationId, ReportId,
};
use crate::core::time::SimTime;
use crate::delegation::MandateAuthority;
use crate::finance::Money;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalRepresentationStatus {
    Active,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalRepresentationEndReason {
    MatterConcluded,
    Replaced,
    SponsorWithdrawn,
    CounselUnavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationParties {
    pub(in crate::legal) arrest: ArrestId,
    pub(in crate::legal) defendant: CharacterId,
    pub(in crate::legal) sponsor: OrganizationId,
    pub(in crate::legal) counsel: CharacterId,
    pub(in crate::legal) counsel_institution: OrganizationId,
    pub(in crate::legal) contact: ContactId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationPayment {
    pub(in crate::legal) fee: Money,
    pub(in crate::legal) provider_account: FinancialAccountId,
    pub(in crate::legal) payment: LedgerTransactionId,
    pub(in crate::legal) authorization: Option<MandateAuthority>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationLifecycle {
    pub(in crate::legal) retained_at: SimTime,
    pub(in crate::legal) ended_at: Option<SimTime>,
    pub(in crate::legal) end_reason: Option<LegalRepresentationEndReason>,
    pub(in crate::legal) status: LegalRepresentationStatus,
    pub(in crate::legal) origin: super::LegalRepresentationOrigin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationArtifacts {
    pub(in crate::legal) information: InformationId,
    pub(in crate::legal) report: ReportId,
    pub(in crate::legal) ended_information: Option<InformationId>,
    pub(in crate::legal) ended_report: Option<ReportId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegalRepresentationRecord {
    pub(in crate::legal) id: LegalRepresentationId,
    pub(in crate::legal) parties: LegalRepresentationParties,
    pub(in crate::legal) payment: LegalRepresentationPayment,
    pub(in crate::legal) lifecycle: LegalRepresentationLifecycle,
    pub(in crate::legal) artifacts: LegalRepresentationArtifacts,
    pub(in crate::legal) version: u32,
}

impl LegalRepresentationRecord {
    pub fn id(&self) -> LegalRepresentationId {
        self.id
    }

    pub fn arrest(&self) -> ArrestId {
        self.parties.arrest
    }

    pub fn defendant(&self) -> CharacterId {
        self.parties.defendant
    }

    pub fn sponsor(&self) -> OrganizationId {
        self.parties.sponsor
    }

    pub fn counsel(&self) -> CharacterId {
        self.parties.counsel
    }

    pub fn counsel_institution(&self) -> OrganizationId {
        self.parties.counsel_institution
    }

    pub fn contact(&self) -> ContactId {
        self.parties.contact
    }

    pub fn fee(&self) -> Money {
        self.payment.fee
    }

    pub fn provider_account(&self) -> FinancialAccountId {
        self.payment.provider_account
    }

    pub fn payment(&self) -> LedgerTransactionId {
        self.payment.payment
    }

    pub fn authorization(&self) -> Option<MandateAuthority> {
        self.payment.authorization
    }

    pub fn retained_at(&self) -> SimTime {
        self.lifecycle.retained_at
    }

    pub fn ended_at(&self) -> Option<SimTime> {
        self.lifecycle.ended_at
    }

    pub fn end_reason(&self) -> Option<LegalRepresentationEndReason> {
        self.lifecycle.end_reason
    }

    pub fn status(&self) -> LegalRepresentationStatus {
        self.lifecycle.status
    }

    pub fn origin(&self) -> super::LegalRepresentationOrigin {
        self.lifecycle.origin
    }

    pub fn information(&self) -> InformationId {
        self.artifacts.information
    }

    pub fn report(&self) -> ReportId {
        self.artifacts.report
    }

    pub fn ended_information(&self) -> Option<InformationId> {
        self.artifacts.ended_information
    }

    pub fn ended_report(&self) -> Option<ReportId> {
        self.artifacts.ended_report
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

/// Who initiated a legal representation. The automatic-support governance pass may only end
/// representations it retained itself; an explicitly commanded retention ends through player
/// or delegated decisions, never the custody sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalRepresentationOrigin {
    AutomaticPolicy,
    DirectRetention,
}

#[derive(Clone, Debug)]
pub struct LegalRepresentationDraft {
    pub arrest: ArrestId,
    pub sponsor: OrganizationId,
    pub contact: ContactId,
    pub fee: Money,
    /// Sponsor-owned liquid accounts permitted to fund the retainer. Direct and
    /// organization-policy automatic retention may aggregate them; mandate-sourced automatic
    /// and explicitly delegated retention are constrained to the mandate's one budget funding
    /// account by the canonical payment validator.
    pub payer_accounts: BTreeSet<FinancialAccountId>,
    pub provider_account: FinancialAccountId,
    pub authorization: Option<MandateAuthority>,
    pub origin: LegalRepresentationOrigin,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct LegalRepresentationIndexes {
    pub(in crate::legal) active_by_arrest: BTreeMap<ArrestId, LegalRepresentationId>,
    pub(in crate::legal) active_by_contact: BTreeMap<ContactId, BTreeSet<LegalRepresentationId>>,
    /// Active automatic-policy retentions, so the per-tick custody sweep concludes only its
    /// own matters without scanning the full representation history, which grows forever.
    pub(in crate::legal) active_automatic_policy: BTreeSet<LegalRepresentationId>,
}
