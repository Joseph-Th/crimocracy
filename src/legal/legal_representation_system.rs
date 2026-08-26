//! Retained legal counsel transactions backed by real contacts, capabilities, and ledger payments.

use crate::contacts::{ContactKind, ContactStatus};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, ContactId, FinancialAccountId, IdExhaustionError, IdKind,
    LegalRepresentationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::delegation::delegation_system::{
    resolve_mandate_authority, resolve_policy_for_manager, DelegationError,
};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
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
use crate::legal::{
    ArrestStatus, LegalRepresentationDraft, LegalRepresentationEndReason,
    LegalRepresentationRecord, LegalRepresentationStatus,
};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LegalRepresentationError {
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("defendant {0} does not exist")]
    MissingDefendant(CharacterId),
    #[error("defendant {defendant} is not an active member of sponsor {sponsor}")]
    InvalidDefendantMembership {
        defendant: CharacterId,
        sponsor: OrganizationId,
    },
    #[error("sponsor organization {0} does not exist")]
    MissingSponsor(OrganizationId),
    #[error("organization {0} is not an active criminal sponsor")]
    InvalidSponsor(OrganizationId),
    #[error("institutional contact {0} does not exist")]
    MissingContact(ContactId),
    #[error("institutional contact {0} is not active")]
    InactiveContact(ContactId),
    #[error("institutional contact {contact} belongs to sponsor {actual}, not {expected}")]
    ContactSponsorMismatch {
        contact: ContactId,
        expected: OrganizationId,
        actual: OrganizationId,
    },
    #[error("institutional contact {0} is not a legal-services channel")]
    ContactNotLegal(ContactId),
    #[error("contact handler {0} does not exist")]
    MissingHandler(CharacterId),
    #[error("contact handler {handler} is not available to sponsor {sponsor}")]
    UnavailableHandler {
        handler: CharacterId,
        sponsor: OrganizationId,
    },
    #[error("counsel character {0} does not exist")]
    MissingCounsel(CharacterId),
    #[error("counsel character {0} is detained and cannot accept a new representation")]
    DetainedCounsel(CharacterId),
    #[error("counsel character {0} has no LegalKnowledge capability")]
    MissingLegalKnowledge(CharacterId),
    #[error("counsel institution {0} does not exist")]
    MissingCounselInstitution(OrganizationId),
    #[error("counsel institution {0} is not an active legal-services organization")]
    InvalidCounselInstitution(OrganizationId),
    #[error("arrest {arrest} already has active representation {representation}")]
    AlreadyRepresented {
        arrest: ArrestId,
        representation: LegalRepresentationId,
    },
    #[error("arrest {arrest} is not an active detention")]
    ArrestNotActive { arrest: ArrestId },
    #[error("legal retainer fee must be greater than zero")]
    InvalidFee,
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("payer account {account} is not a liquid account owned by sponsor {sponsor}")]
    InvalidPayerAccount {
        account: FinancialAccountId,
        sponsor: OrganizationId,
    },
    #[error("provider account {account} is not a legitimate operating account owned by legal-services organization {provider}")]
    InvalidProviderAccount {
        account: FinancialAccountId,
        provider: OrganizationId,
    },
    #[error("payer account {account} has {available_cents} cents but retainer requires {required_cents} cents")]
    InsufficientFunds {
        account: FinancialAccountId,
        available_cents: i64,
        required_cents: i64,
    },
    #[error("legal retainer fee cannot be represented as a balanced ledger outflow")]
    FeeArithmeticOverflow,
    #[error("delegated legal representation authority must use the Legal responsibility function")]
    InvalidAuthorityScope,
    #[error("delegated legal representation authority belongs to organization {actual}, not sponsor {expected}")]
    AuthorityOrganizationMismatch {
        expected: OrganizationId,
        actual: OrganizationId,
    },
    #[error("legal representation validation was performed at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("arrest {arrest} changed after legal representation validation; expected version {expected}, found {found}")]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error("contact {contact} changed after legal representation validation; expected version {expected}, found {found}")]
    StaleContact {
        contact: ContactId,
        expected: u32,
        found: u32,
    },
    #[error("defendant {defendant} changed after legal representation validation; expected version {expected}, found {found}")]
    StaleDefendant {
        defendant: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("counsel {counsel} changed after legal representation validation; expected version {expected}, found {found}")]
    StaleCounsel {
        counsel: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("contact handler {handler} changed after legal representation validation; expected version {expected}, found {found}")]
    StaleHandler {
        handler: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("legal representation {0} does not exist")]
    MissingRepresentation(LegalRepresentationId),
    #[error("legal representation {0} is not active")]
    RepresentationNotActive(LegalRepresentationId),
    #[error("legal representation {representation} changed after end validation; expected version {expected}, found {found}")]
    StaleRepresentation {
        representation: LegalRepresentationId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Copy, Debug)]
struct RepresentationDependencies {
    defendant: CharacterId,
    counsel: CharacterId,
    counsel_institution: OrganizationId,
    handler: CharacterId,
    arrest_version: u32,
    defendant_version: u32,
    counsel_version: u32,
    handler_version: u32,
    contact_version: u32,
}

pub struct ValidatedLegalRepresentation {
    draft: LegalRepresentationDraft,
    dependencies: RepresentationDependencies,
    retained_at: SimTime,
    payment: ValidatedLedgerTransaction,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedLegalRepresentation {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<LegalRepresentationId, LegalRepresentationError> {
        state.ids.reserve_many(&[
            (IdKind::LedgerTransaction, 1),
            (IdKind::Information, 1),
            (IdKind::Report, 1),
            (IdKind::LegalRepresentation, 1),
        ])?;
        validate_time(state, self.retained_at)?;
        validate_dependency_versions(
            state,
            self.draft.arrest,
            self.draft.contact,
            self.dependencies,
        )?;
        let current = validate_representation_dependencies(state, &self.draft)?;
        if current.defendant != self.dependencies.defendant
            || current.counsel != self.dependencies.counsel
            || current.counsel_institution != self.dependencies.counsel_institution
            || current.handler != self.dependencies.handler
        {
            return Err(LegalRepresentationError::StaleContact {
                contact: self.draft.contact,
                expected: self.dependencies.contact_version,
                found: current.contact_version,
            });
        }

        let payment = self.payment.commit(state)?;
        let information = self
            .information
            .commit(state)
            .expect("retainer information ID was preflighted before payment mutation");
        let report = self
            .report
            .commit(state)
            .expect("retainer report ID was preflighted before payment mutation");
        let id = state
            .ids
            .next_legal_representation()
            .expect("legal-representation ID was preflighted before payment mutation");
        state
            .legal
            .insert_legal_representation(LegalRepresentationRecord {
                id,
                parties: super::LegalRepresentationParties {
                    arrest: self.draft.arrest,
                    defendant: self.dependencies.defendant,
                    sponsor: self.draft.sponsor,
                    counsel: self.dependencies.counsel,
                    counsel_institution: self.dependencies.counsel_institution,
                    contact: self.draft.contact,
                },
                payment: super::LegalRepresentationPayment {
                    fee: self.draft.fee,
                    payer_account: self.draft.payer_account,
                    provider_account: self.draft.provider_account,
                    payment,
                    authorization: self.draft.authorization,
                },
                lifecycle: super::LegalRepresentationLifecycle {
                    retained_at: self.retained_at,
                    ended_at: None,
                    end_reason: None,
                    status: LegalRepresentationStatus::Active,
                    origin: self.draft.origin,
                },
                artifacts: super::LegalRepresentationArtifacts {
                    information,
                    report,
                    ended_information: None,
                    ended_report: None,
                },
                version: 1,
            });
        Ok(id)
    }
}

pub fn validate_retain_legal_representation(
    state: &AppState,
    draft: LegalRepresentationDraft,
) -> Result<ValidatedLegalRepresentation, LegalRepresentationError> {
    let dependencies = validate_representation_dependencies(state, &draft)?;
    let retained_at = state.now();
    let payment = validate_retainer_payment(state, &draft, dependencies.counsel_institution)?;
    let defendant = state
        .world
        .get_character(dependencies.defendant)
        .expect("validated legal representation defendant must exist");
    let counsel = state
        .world
        .get_character(dependencies.counsel)
        .expect("validated legal counsel must exist");
    let firm = state
        .world
        .get_organization(dependencies.counsel_institution)
        .expect("validated legal-services institution must exist");

    let summary = format!(
        "{} retained {} of {} to represent {} for a fee of {}.",
        state
            .world
            .get_organization(draft.sponsor)
            .expect("validated sponsor must exist")
            .name(),
        counsel.name(),
        firm.name(),
        defendant.name(),
        crate::finance::helpers::format_money_cents(draft.fee.cents()),
    );
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(draft.sponsor),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(dependencies.counsel)),
            subject: EntityRef::Character(dependencies.defendant),
            observed_at: retained_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: draft.sponsor,
            kind: ReportKind::Legal,
            title: "Legal representation retained".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(dependencies.defendant),
                    EntityRef::Character(dependencies.counsel),
                    EntityRef::Organization(dependencies.counsel_institution),
                    EntityRef::Investigation(
                        state
                            .legal
                            .get_arrest(draft.arrest)
                            .expect("validated arrest must exist")
                            .investigation(),
                    ),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedLegalRepresentation {
        draft,
        dependencies,
        retained_at,
        payment,
        information,
        report,
    })
}

fn validate_representation_dependencies(
    state: &AppState,
    draft: &LegalRepresentationDraft,
) -> Result<RepresentationDependencies, LegalRepresentationError> {
    let arrest = state
        .legal
        .get_arrest(draft.arrest)
        .ok_or(LegalRepresentationError::MissingArrest(draft.arrest))?;
    if arrest.status() != ArrestStatus::Detained {
        return Err(LegalRepresentationError::ArrestNotActive {
            arrest: draft.arrest,
        });
    }
    if let Some(existing) = state.legal.active_representation_for_arrest(draft.arrest) {
        return Err(LegalRepresentationError::AlreadyRepresented {
            arrest: draft.arrest,
            representation: existing.id(),
        });
    }

    let sponsor = state
        .world
        .get_organization(draft.sponsor)
        .ok_or(LegalRepresentationError::MissingSponsor(draft.sponsor))?;
    if sponsor.kind() != OrganizationKind::Criminal {
        return Err(LegalRepresentationError::InvalidSponsor(draft.sponsor));
    }

    let defendant = state.world.get_character(arrest.character()).ok_or(
        LegalRepresentationError::MissingDefendant(arrest.character()),
    )?;
    if defendant.organization() != Some(draft.sponsor) {
        return Err(LegalRepresentationError::InvalidDefendantMembership {
            defendant: arrest.character(),
            sponsor: draft.sponsor,
        });
    }

    let contact = state
        .contacts
        .get_contact(draft.contact)
        .ok_or(LegalRepresentationError::MissingContact(draft.contact))?;
    if contact.status() != ContactStatus::Active {
        return Err(LegalRepresentationError::InactiveContact(draft.contact));
    }
    if contact.sponsor() != draft.sponsor {
        return Err(LegalRepresentationError::ContactSponsorMismatch {
            contact: draft.contact,
            expected: draft.sponsor,
            actual: contact.sponsor(),
        });
    }
    if contact.kind() != ContactKind::Legal {
        return Err(LegalRepresentationError::ContactNotLegal(draft.contact));
    }

    let handler = state
        .world
        .get_character(contact.handler())
        .ok_or(LegalRepresentationError::MissingHandler(contact.handler()))?;
    if handler.organization() != Some(draft.sponsor)
        || state
            .legal
            .active_arrest_for_character(contact.handler())
            .is_some()
    {
        return Err(LegalRepresentationError::UnavailableHandler {
            handler: contact.handler(),
            sponsor: draft.sponsor,
        });
    }

    let counsel = state
        .world
        .get_character(contact.contact())
        .ok_or(LegalRepresentationError::MissingCounsel(contact.contact()))?;
    if state
        .legal
        .active_arrest_for_character(contact.contact())
        .is_some()
    {
        return Err(LegalRepresentationError::DetainedCounsel(contact.contact()));
    }
    if counsel.capability(CapabilityKind::LegalKnowledge).is_none() {
        return Err(LegalRepresentationError::MissingLegalKnowledge(
            contact.contact(),
        ));
    }
    if counsel.organization() != Some(contact.institution()) {
        return Err(LegalRepresentationError::InvalidCounselInstitution(
            contact.institution(),
        ));
    }
    let institution = state.world.get_organization(contact.institution()).ok_or(
        LegalRepresentationError::MissingCounselInstitution(contact.institution()),
    )?;
    if institution.kind() != OrganizationKind::LegalServices {
        return Err(LegalRepresentationError::InvalidCounselInstitution(
            contact.institution(),
        ));
    }

    validate_authority(state, draft.sponsor, draft.authorization)?;

    Ok(RepresentationDependencies {
        defendant: arrest.character(),
        counsel: contact.contact(),
        counsel_institution: contact.institution(),
        handler: contact.handler(),
        arrest_version: arrest.version(),
        defendant_version: defendant.version(),
        counsel_version: counsel.version(),
        handler_version: handler.version(),
        contact_version: contact.version(),
    })
}

fn validate_authority(
    state: &AppState,
    sponsor: OrganizationId,
    authorization: Option<MandateAuthority>,
) -> Result<(), LegalRepresentationError> {
    let Some(authority) = authorization else {
        return Ok(());
    };
    if authority.scope != ResponsibilityScope::Function(ResponsibilityFunction::Legal) {
        return Err(LegalRepresentationError::InvalidAuthorityScope);
    }
    let resolved = resolve_mandate_authority(state, authority)?;
    if resolved.organization() != sponsor {
        return Err(LegalRepresentationError::AuthorityOrganizationMismatch {
            expected: sponsor,
            actual: resolved.organization(),
        });
    }
    Ok(())
}

fn validate_retainer_payment(
    state: &AppState,
    draft: &LegalRepresentationDraft,
    provider: OrganizationId,
) -> Result<ValidatedLedgerTransaction, LegalRepresentationError> {
    if draft.fee <= Money::ZERO {
        return Err(LegalRepresentationError::InvalidFee);
    }
    let payer = state.finance.get_account(draft.payer_account).ok_or(
        LegalRepresentationError::MissingAccount(draft.payer_account),
    )?;
    if payer.owner() != FinancialOwner::Organization(draft.sponsor)
        || !matches!(
            payer.kind(),
            AccountKind::StreetCash
                | AccountKind::ConcealedCash
                | AccountKind::AccountedFunds
                | AccountKind::LegitimateOperating
        )
    {
        return Err(LegalRepresentationError::InvalidPayerAccount {
            account: draft.payer_account,
            sponsor: draft.sponsor,
        });
    }
    if payer.balance() < draft.fee {
        return Err(LegalRepresentationError::InsufficientFunds {
            account: draft.payer_account,
            available_cents: payer.balance().cents(),
            required_cents: draft.fee.cents(),
        });
    }

    let provider_account = state.finance.get_account(draft.provider_account).ok_or(
        LegalRepresentationError::MissingAccount(draft.provider_account),
    )?;
    if provider_account.owner() != FinancialOwner::Organization(provider)
        || provider_account.kind() != AccountKind::LegitimateOperating
    {
        return Err(LegalRepresentationError::InvalidProviderAccount {
            account: draft.provider_account,
            provider,
        });
    }
    let outflow = draft
        .fee
        .cents()
        .checked_neg()
        .map(Money::from_cents)
        .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?;
    Ok(validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!("Legal retainer for arrest {}", draft.arrest),
            postings: vec![
                LedgerPosting {
                    account: draft.payer_account,
                    amount: outflow,
                },
                LedgerPosting {
                    account: draft.provider_account,
                    amount: draft.fee,
                },
            ],
            authorization: draft.authorization,
        },
    )?)
}

fn validate_dependency_versions(
    state: &AppState,
    arrest: ArrestId,
    contact: ContactId,
    expected: RepresentationDependencies,
) -> Result<(), LegalRepresentationError> {
    let arrest_record = state
        .legal
        .get_arrest(arrest)
        .ok_or(LegalRepresentationError::MissingArrest(arrest))?;
    if arrest_record.version() != expected.arrest_version {
        return Err(LegalRepresentationError::StaleArrest {
            arrest,
            expected: expected.arrest_version,
            found: arrest_record.version(),
        });
    }
    let contact_record = state
        .contacts
        .get_contact(contact)
        .ok_or(LegalRepresentationError::MissingContact(contact))?;
    if contact_record.version() != expected.contact_version {
        return Err(LegalRepresentationError::StaleContact {
            contact,
            expected: expected.contact_version,
            found: contact_record.version(),
        });
    }
    let defendant = state.world.get_character(expected.defendant).ok_or(
        LegalRepresentationError::MissingDefendant(expected.defendant),
    )?;
    if defendant.version() != expected.defendant_version {
        return Err(LegalRepresentationError::StaleDefendant {
            defendant: expected.defendant,
            expected: expected.defendant_version,
            found: defendant.version(),
        });
    }
    let counsel = state
        .world
        .get_character(expected.counsel)
        .ok_or(LegalRepresentationError::MissingCounsel(expected.counsel))?;
    if counsel.version() != expected.counsel_version {
        return Err(LegalRepresentationError::StaleCounsel {
            counsel: expected.counsel,
            expected: expected.counsel_version,
            found: counsel.version(),
        });
    }
    let handler = state
        .world
        .get_character(expected.handler)
        .ok_or(LegalRepresentationError::MissingHandler(expected.handler))?;
    if handler.version() != expected.handler_version {
        return Err(LegalRepresentationError::StaleHandler {
            handler: expected.handler,
            expected: expected.handler_version,
            found: handler.version(),
        });
    }
    Ok(())
}

fn validate_time(state: &AppState, expected: SimTime) -> Result<(), LegalRepresentationError> {
    crate::core::time::ensure_time_current(state.now(), expected)
        .map_err(|(expected, found)| LegalRepresentationError::StaleTime { expected, found })
}

pub struct ValidatedLegalRepresentationEnd {
    representation: LegalRepresentationId,
    reason: LegalRepresentationEndReason,
    expected_version: u32,
    ended_at: SimTime,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedLegalRepresentationEnd {
    pub fn commit(self, state: &mut AppState) -> Result<(), LegalRepresentationError> {
        state
            .ids
            .reserve_many(&[(IdKind::Information, 1), (IdKind::Report, 1)])?;
        validate_time(state, self.ended_at)?;
        let record = state
            .legal
            .get_legal_representation(self.representation)
            .ok_or(LegalRepresentationError::MissingRepresentation(
                self.representation,
            ))?;
        if record.version() != self.expected_version {
            return Err(LegalRepresentationError::StaleRepresentation {
                representation: self.representation,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != LegalRepresentationStatus::Active {
            return Err(LegalRepresentationError::RepresentationNotActive(
                self.representation,
            ));
        }
        let information = self
            .information
            .commit(state)
            .expect("representation-end information ID was preflighted before mutation");
        let report = self
            .report
            .commit(state)
            .expect("representation-end report ID was preflighted before mutation");
        state.legal.end_legal_representation(
            self.representation,
            self.ended_at,
            self.reason,
            information,
            report,
        );
        Ok(())
    }
}

pub fn validate_end_legal_representation(
    state: &AppState,
    representation: LegalRepresentationId,
    reason: LegalRepresentationEndReason,
) -> Result<ValidatedLegalRepresentationEnd, LegalRepresentationError> {
    let record = state.legal.get_legal_representation(representation).ok_or(
        LegalRepresentationError::MissingRepresentation(representation),
    )?;
    if record.status() != LegalRepresentationStatus::Active {
        return Err(LegalRepresentationError::RepresentationNotActive(
            representation,
        ));
    }
    let ended_at = state.now();
    let defendant = state.world.get_character(record.defendant()).ok_or(
        LegalRepresentationError::MissingDefendant(record.defendant()),
    )?;
    let counsel = state
        .world
        .get_character(record.counsel())
        .ok_or(LegalRepresentationError::MissingCounsel(record.counsel()))?;
    let summary = format!(
        "{}'s representation of {} ended: {}.",
        counsel.name(),
        defendant.name(),
        end_reason_label(reason),
    );
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(record.sponsor()),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(record.counsel())),
            subject: EntityRef::Character(record.defendant()),
            observed_at: ended_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.sponsor(),
            kind: ReportKind::Legal,
            title: "Legal representation ended".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(record.defendant()),
                    EntityRef::Character(record.counsel()),
                    EntityRef::Organization(record.counsel_institution()),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedLegalRepresentationEnd {
        representation,
        reason,
        expected_version: record.version(),
        ended_at,
        information,
        report,
    })
}

const fn end_reason_label(reason: LegalRepresentationEndReason) -> &'static str {
    match reason {
        LegalRepresentationEndReason::MatterConcluded => "matter concluded",
        LegalRepresentationEndReason::Replaced => "counsel replaced",
        LegalRepresentationEndReason::SponsorWithdrawn => "sponsor withdrew support",
        LegalRepresentationEndReason::CounselWithdrawn => "counsel withdrew",
    }
}

/// Flat retainer the organization commits when its standing policy promises automatic legal
/// support. A flat authored fee keeps the automatic path honest: no discretionary spending
/// is made in the organization's name beyond what the policy promises.
const AUTOMATIC_SUPPORT_RETAINER_CENTS: i64 = 5_000;

/// Executes `AssociateLegalSupport(Automatic)` governance: every detained member of an
/// organization that runs the Automatic policy gets counsel retained through the canonical
/// representation path, paid from the organization's first funded cash account through its
/// first active Legal-channel contact. Organizations without those prerequisites see no
/// action — the policy promises support, and this stage delivers it only when the pieces
/// exist for the canonical transaction to carry it.
pub fn apply_automatic_legal_support(
    state: &mut AppState,
) -> Result<Vec<crate::core::id::LegalRepresentationId>, LegalRepresentationError> {
    use crate::finance::{AccountKind, FinancialOwner};
    use crate::world::{OrganizationKind as OrgKind, PolicyKind, PolicySetting};

    // Automatic support concludes when the matter it covers does: a representation this pass
    // retained for a detainee who has left custody ends with `MatterConcluded`, so concluded
    // matters stop blocking contact termination. A contact may carry several concurrent
    // matters — counsel serves multiple clients — but it cannot be terminated while any of
    // them is still active.
    // Explicitly commanded retentions are never swept here — their matter is leadership's
    // to end, not governance's.
    //
    // Both halves of this stage need custody work to observe: the sweep ends automatic
    // representations whose detainee left, and retention needs a detained member. With no
    // representation and nobody in custody there is nothing this pass could do, so quiet
    // ticks skip both scans.
    if !state.legal.has_active_automatic_policy_representations()
        && !state.legal.has_detained_arrests()
    {
        return Ok(Vec::new());
    }
    let concluded: Vec<LegalRepresentationId> = state
        .legal
        .active_automatic_policy_representations()
        .filter(|record| {
            state
                .legal
                .get_arrest(record.arrest())
                .is_none_or(|arrest| arrest.status() != ArrestStatus::Detained)
        })
        .map(|record| record.id())
        .collect();
    for representation in concluded {
        // An autonomous stage must not abort the tick on one drifted record; the same
        // canonical end path a player command would use stays in charge of each ending.
        if let Ok(token) = validate_end_legal_representation(
            state,
            representation,
            LegalRepresentationEndReason::MatterConcluded,
        ) {
            token.commit(state).ok();
        }
    }

    let candidates: Vec<crate::core::id::ArrestId> = state
        .legal
        .detained_arrests()
        .filter(|arrest| {
            state
                .legal
                .active_representation_for_arrest(arrest.id())
                .is_none()
        })
        .filter_map(|arrest| {
            let defendant = arrest.character();
            let defendant_record = state.world.get_character(defendant)?;
            let organization = defendant_record.organization()?;
            let record = state.world.get_organization(organization)?;
            if record.kind() != OrgKind::Criminal {
                return None;
            }
            // The mandate standing order of whoever supervises the detained associate
            // governs first — delegation overrides are real policy, not decoration. Without
            // a resolvable supervisor the organization default applies.
            let setting = defendant_record
                .supervisor()
                .and_then(|supervisor| {
                    resolve_policy_for_manager(state, supervisor, PolicyKind::AssociateLegalSupport)
                        .ok()
                })
                .map(|resolved| resolved.setting)
                .or_else(|| record.policy(PolicyKind::AssociateLegalSupport));
            matches!(
                setting,
                Some(PolicySetting::AssociateLegalSupport(
                    crate::world::LegalSupportPolicy::Automatic,
                )),
            )
            .then_some(arrest.id())
        })
        .collect();

    let mut retained = Vec::new();
    for arrest_id in candidates {
        let arrest = state
            .legal
            .get_arrest(arrest_id)
            .expect("indexed detained arrest must exist");
        let defendant = arrest.character();
        let Some(sponsor) = state
            .world
            .get_character(defendant)
            .and_then(|record| record.organization())
        else {
            continue;
        };
        // First active Legal-channel contact of the sponsor, by contact id order, with its
        // institution captured in the same pass.
        let fee = crate::finance::Money::from_cents(AUTOMATIC_SUPPORT_RETAINER_CENTS);
        let Some((contact, institution)) = state
            .contacts
            .contacts_for_sponsor(sponsor)
            .find(|contact| {
                contact.status() == crate::contacts::ContactStatus::Active
                    && contact.kind() == crate::contacts::ContactKind::Legal
            })
            .map(|contact| (contact.id(), contact.institution()))
        else {
            continue;
        };

        // The payer must be a sponsor-owned cash account that can cover the flat retainer;
        // the provider account is the counsel institution's operating account.
        let payer_account = state
            .finance
            .accounts_for(FinancialOwner::Organization(sponsor))
            .find(|account| {
                matches!(
                    account.kind(),
                    AccountKind::StreetCash
                        | AccountKind::ConcealedCash
                        | AccountKind::AccountedFunds
                        | AccountKind::LegitimateOperating
                ) && account.balance() >= fee
            });
        let Some(payer_account) = payer_account else {
            continue;
        };
        let provider_account = state
            .finance
            .accounts_for(FinancialOwner::Organization(institution))
            .find(|account| account.kind() == AccountKind::LegitimateOperating);
        let Some(provider_account) = provider_account else {
            continue;
        };

        let draft = LegalRepresentationDraft {
            arrest: arrest_id,
            sponsor,
            contact,
            fee,
            payer_account: payer_account.id(),
            provider_account: provider_account.id(),
            authorization: None,
            origin: crate::legal::LegalRepresentationOrigin::AutomaticPolicy,
        };
        if validate_representation_dependencies(state, &draft).is_err() {
            // Prerequisites drifted since the snapshot (contact inactive, custody released);
            // the policy stage simply waits rather than forcing a partial transaction.
            continue;
        }
        // A candidate whose prerequisites changed between the dependency check and commit is
        // skipped, not fatal: an autonomous pass must never abort the tick.
        if let Ok(representation) = validate_retain_legal_representation(state, draft)
            .and_then(|validated| validated.commit(state))
        {
            retained.push(representation);
        }
    }
    Ok(retained)
}

#[cfg(test)]
mod tests;
