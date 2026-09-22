//! Retained legal counsel transactions backed by real contacts, capabilities, and ledger payments,
//! including canonical representation closure when counsel becomes unavailable through custody.

use crate::contacts::{ContactKind, ContactStatus};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, ContactId, FinancialAccountId, IdExhaustionError, IdKind,
    LegalRepresentationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::delegation::delegation_system::{DelegationError, resolve_mandate_authority};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_record_transaction,
};
use crate::finance::{AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_system_information,
    validate_record_system_information_with_signal,
};
use crate::intelligence::{
    InformationDraft, InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder,
    LegalPersonStatusSignal, Reliability, Specificity,
};
use crate::legal::{
    ArrestStatus, LegalRepresentationDraft, LegalRepresentationEndReason,
    LegalRepresentationRecord, LegalRepresentationStatus,
};
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
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
    #[error(
        "provider account {account} is not a legitimate operating account owned by legal-services organization {provider}"
    )]
    InvalidProviderAccount {
        account: FinancialAccountId,
        provider: OrganizationId,
    },
    #[error(
        "sponsor liquid accounts have {available_cents} cents but retainer requires {required_cents} cents"
    )]
    InsufficientFunds {
        available_cents: i64,
        required_cents: i64,
    },
    #[error("legal retainer must name at least one sponsor funding account")]
    NoPayerAccounts,
    #[error("delegated legal retention must be funded only from mandate budget account {required}")]
    DelegatedFundingAccountMismatch { required: FinancialAccountId },
    #[error("legal retainer fee cannot be represented as a balanced ledger outflow")]
    FeeArithmeticOverflow,
    #[error("delegated legal representation authority must use the Legal responsibility function")]
    InvalidAuthorityScope,
    #[error(
        "delegated legal representation authority belongs to organization {actual}, not sponsor {expected}"
    )]
    AuthorityOrganizationMismatch {
        expected: OrganizationId,
        actual: OrganizationId,
    },
    #[error(
        "legal representation validation was performed at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(
        "arrest {arrest} changed after legal representation validation; expected version {expected}, found {found}"
    )]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error("counsel {counsel}'s active representations changed after detention preflight")]
    DetentionRepresentationsChanged { counsel: CharacterId },
    #[error(
        "contact {contact} changed after legal representation validation; expected version {expected}, found {found}"
    )]
    StaleContact {
        contact: ContactId,
        expected: u32,
        found: u32,
    },
    #[error(
        "defendant {defendant} changed after legal representation validation; expected version {expected}, found {found}"
    )]
    StaleDefendant {
        defendant: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "counsel {counsel} changed after legal representation validation; expected version {expected}, found {found}"
    )]
    StaleCounsel {
        counsel: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "contact handler {handler} changed after legal representation validation; expected version {expected}, found {found}"
    )]
    StaleHandler {
        handler: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("legal representation {0} does not exist")]
    MissingRepresentation(LegalRepresentationId),
    #[error("legal representation {0} is not active")]
    RepresentationNotActive(LegalRepresentationId),
    #[error(
        "legal representation {representation} changed after end validation; expected version {expected}, found {found}"
    )]
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
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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
        let mut budget = self.payment.id_budget();
        budget.extend([
            (IdKind::Information, 1),
            (IdKind::Report, 1),
            (IdKind::LegalRepresentation, 1),
        ]);
        state.ids.reserve_many(&budget)?;
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

    let summary = retained_representation_summary(
        state
            .world
            .get_organization(draft.sponsor)
            .expect("validated sponsor must exist")
            .name(),
        counsel.name(),
        firm.name(),
        defendant.name(),
        draft.fee,
    );
    let information = validate_record_system_information_with_signal(
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
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
            arrest: draft.arrest,
        }),
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
    if draft.payer_accounts.is_empty() {
        return Err(LegalRepresentationError::NoPayerAccounts);
    }
    if let Some(authority) = draft.authorization {
        let mandate = state
            .delegation
            .get_mandate(authority.mandate)
            .ok_or(FinanceError::MissingMandate(authority.mandate))?;
        let funding_account = mandate
            .budget()
            .ok_or(FinanceError::MissingBudget(authority.mandate))?
            .funding_account;
        if draft.payer_accounts.len() != 1 || !draft.payer_accounts.contains(&funding_account) {
            return Err(LegalRepresentationError::DelegatedFundingAccountMismatch {
                required: funding_account,
            });
        }
    }

    let mut available_cents = 0_i128;
    let mut payers = Vec::with_capacity(draft.payer_accounts.len());
    for account_id in &draft.payer_accounts {
        let payer = state
            .finance
            .get_account(*account_id)
            .ok_or(LegalRepresentationError::MissingAccount(*account_id))?;
        if payer.owner() != FinancialOwner::Organization(draft.sponsor) || !payer.kind().is_liquid()
        {
            return Err(LegalRepresentationError::InvalidPayerAccount {
                account: *account_id,
                sponsor: draft.sponsor,
            });
        }
        let spendable = payer.spendable_balance();
        available_cents =
            (available_cents + i128::from(spendable.cents())).min(i128::from(draft.fee.cents()));
        if spendable > Money::ZERO {
            payers.push((
                payer.kind().unrestricted_spending_priority(),
                spendable,
                payer.id(),
            ));
        }
    }
    if available_cents < i128::from(draft.fee.cents()) {
        return Err(LegalRepresentationError::InsufficientFunds {
            available_cents: i64::try_from(available_cents)
                .expect("available retainer funding is bounded by fee"),
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
    // Any-liquid retainer funding follows the finance owner's shared money-state semantics
    // instead of account creation order. Within one money-state class, consume the largest pool
    // first to keep the balanced transaction compact; account ID is only the final stable tie.
    payers.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.cmp(&right.2))
    });
    let mut postings = Vec::with_capacity(draft.payer_accounts.len() + 1);
    let mut remaining = draft.fee;
    for (_, spendable, account_id) in payers {
        if remaining == Money::ZERO {
            break;
        }
        let debit = spendable.min(remaining);
        postings.push(LedgerPosting {
            account: account_id,
            amount: debit
                .checked_neg()
                .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?,
        });
        remaining = remaining
            .checked_sub(debit)
            .ok_or(LegalRepresentationError::FeeArithmeticOverflow)?;
    }
    debug_assert_eq!(remaining, Money::ZERO);
    postings.push(LedgerPosting {
        account: draft.provider_account,
        amount: draft.fee,
    });
    Ok(validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!("Legal retainer for arrest {}", draft.arrest),
            postings,
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
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        vec![(IdKind::Information, 1), (IdKind::Report, 1)]
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), LegalRepresentationError> {
        state.ids.reserve_many(&self.id_budget())?;
        self.ensure_current(state)?;
        self.commit_preflighted(state);
        Ok(())
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), LegalRepresentationError> {
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
        ensure_version_can_advance(record.version(), "legal representation")?;
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
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
    }
}

/// Legal-representation side of custody preemption. The wrapper exists even when a counsel has
/// no active matters so an arrest token also stales if representation is retained after arrest
/// validation but before commit.
pub(crate) struct ValidatedCounselDetentionEnds {
    counsel: CharacterId,
    representations: Vec<ValidatedLegalRepresentationEnd>,
}

impl std::fmt::Debug for ValidatedCounselDetentionEnds {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedCounselDetentionEnds")
            .field("counsel", &self.counsel)
            .field(
                "representations",
                &self
                    .representations
                    .iter()
                    .map(|validated| validated.representation)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ValidatedCounselDetentionEnds {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), LegalRepresentationError> {
        let current = active_representation_ids_for_counsel(state, self.counsel);
        let expected: Vec<_> = self
            .representations
            .iter()
            .map(|validated| validated.representation)
            .collect();
        if current != expected {
            return Err(LegalRepresentationError::DetentionRepresentationsChanged {
                counsel: self.counsel,
            });
        }
        for representation in &self.representations {
            representation.ensure_current(state)?;
        }
        Ok(())
    }

    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        self.representations
            .iter()
            .flat_map(ValidatedLegalRepresentationEnd::id_budget)
            .collect()
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        for representation in self.representations {
            representation.commit_preflighted(state);
        }
    }
}

pub(crate) fn validate_end_representations_for_counsel_detention(
    state: &AppState,
    counsel: CharacterId,
) -> Result<ValidatedCounselDetentionEnds, LegalRepresentationError> {
    let representations = active_representation_ids_for_counsel(state, counsel)
        .into_iter()
        .map(|representation| {
            validate_end_legal_representation(
                state,
                representation,
                LegalRepresentationEndReason::CounselUnavailable,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ValidatedCounselDetentionEnds {
        counsel,
        representations,
    })
}

fn active_representation_ids_for_counsel(
    state: &AppState,
    counsel: CharacterId,
) -> Vec<LegalRepresentationId> {
    state
        .contacts()
        .active_contacts_for_character(counsel)
        .flat_map(|contact| {
            state
                .legal()
                .active_representations_for_contact(contact.id())
        })
        .filter(|representation| representation.counsel() == counsel)
        .map(LegalRepresentationRecord::id)
        .collect()
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
    ensure_version_can_advance(record.version(), "legal representation")?;
    let ended_at = state.now();
    let defendant = state.world.get_character(record.defendant()).ok_or(
        LegalRepresentationError::MissingDefendant(record.defendant()),
    )?;
    let counsel = state
        .world
        .get_character(record.counsel())
        .ok_or(LegalRepresentationError::MissingCounsel(record.counsel()))?;
    let summary = ended_representation_summary(counsel.name(), defendant.name(), reason);
    let information = validate_record_system_information(
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

pub(crate) fn retained_representation_summary(
    sponsor: &str,
    counsel: &str,
    firm: &str,
    defendant: &str,
    fee: Money,
) -> String {
    format!(
        "{sponsor} retained {counsel} of {firm} to represent {defendant} for a fee of {}.",
        crate::finance::helpers::format_money_cents(fee.cents()),
    )
}

pub(crate) fn ended_representation_summary(
    counsel: &str,
    defendant: &str,
    reason: LegalRepresentationEndReason,
) -> String {
    format!(
        "{counsel}'s representation of {defendant} ended: {}.",
        end_reason_label(reason),
    )
}

fn end_reason_label(reason: LegalRepresentationEndReason) -> &'static str {
    match reason {
        LegalRepresentationEndReason::MatterConcluded => "matter concluded",
        LegalRepresentationEndReason::Replaced => "counsel replaced",
        LegalRepresentationEndReason::SponsorWithdrawn => "sponsor withdrew support",
        LegalRepresentationEndReason::CounselUnavailable => "counsel became unavailable",
    }
}

mod automatic_support;
pub(crate) use automatic_support::apply_automatic_legal_support;

#[cfg(test)]
mod tests;
