//! Retained legal counsel transactions backed by real contacts, capabilities, and ledger payments.

use crate::contacts::{ContactKind, ContactStatus};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, ContactId, FinancialAccountId, LegalRepresentationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::delegation::delegation_system::{resolve_mandate_authority, DelegationError};
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
    LegalRepresentationDraft, LegalRepresentationEndReason, LegalRepresentationRecord,
    LegalRepresentationStatus,
};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, Lifecycle, OrganizationKind};
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
    #[error("counsel character {0} is not active")]
    InactiveCounsel(CharacterId),
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
        let information = self.information.commit(state);
        let report = self.report.commit(state);
        let id = state.ids.next_legal_representation();
        state
            .legal
            .insert_legal_representation(LegalRepresentationRecord {
                id,
                arrest: self.draft.arrest,
                defendant: self.dependencies.defendant,
                sponsor: self.draft.sponsor,
                counsel: self.dependencies.counsel,
                counsel_institution: self.dependencies.counsel_institution,
                contact: self.draft.contact,
                fee: self.draft.fee,
                payer_account: self.draft.payer_account,
                provider_account: self.draft.provider_account,
                payment,
                authorization: self.draft.authorization,
                retained_at: self.retained_at,
                ended_at: None,
                end_reason: None,
                status: LegalRepresentationStatus::Active,
                information,
                report,
                ended_information: None,
                ended_report: None,
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
        "{} retained {} of {} to represent {} for a fee of {} cents.",
        state
            .world
            .get_organization(draft.sponsor)
            .expect("validated sponsor must exist")
            .name(),
        counsel.name(),
        firm.name(),
        defendant.name(),
        draft.fee.cents(),
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
    if sponsor.lifecycle() != Lifecycle::Active || sponsor.kind() != OrganizationKind::Criminal {
        return Err(LegalRepresentationError::InvalidSponsor(draft.sponsor));
    }

    let defendant = state.world.get_character(arrest.character()).ok_or(
        LegalRepresentationError::MissingDefendant(arrest.character()),
    )?;
    if defendant.lifecycle() != Lifecycle::Active || defendant.organization() != Some(draft.sponsor)
    {
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
    if handler.lifecycle() != Lifecycle::Active
        || handler.organization() != Some(draft.sponsor)
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
    if counsel.lifecycle() != Lifecycle::Active {
        return Err(LegalRepresentationError::InactiveCounsel(contact.contact()));
    }
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
    if institution.lifecycle() != Lifecycle::Active
        || institution.kind() != OrganizationKind::LegalServices
    {
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
    if state.now() != expected {
        return Err(LegalRepresentationError::StaleTime {
            expected,
            found: state.now(),
        });
    }
    Ok(())
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
        let information = self.information.commit(state);
        let report = self.report.commit(state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::contacts::contact_system::{
        validate_establish_contact, validate_terminate_contact, ContactError,
        InstitutionalContactDraft,
    };
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{BudgetAuthority, BudgetPeriod, MandateDraft};
    use crate::finance::finance_system::{insert_account, validate_record_transaction};
    use crate::finance::{FinancialAccountDraft, LedgerPosting};
    use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft,
    };
    use crate::registry::Registry;
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        registry: Registry,
        state: AppState,
        sponsor: OrganizationId,
        handler: CharacterId,
        defendant: CharacterId,
        firm: OrganizationId,
        counsel: CharacterId,
        contact: ContactId,
        arrest: ArrestId,
        payer: FinancialAccountId,
        provider: FinancialAccountId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn level(value: u8) -> RelationshipLevel {
        RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
    }

    fn relationship() -> RelationshipDimensions {
        RelationshipDimensions {
            trust: level(70),
            respect: level(65),
            fear: level(0),
            affection: level(20),
            dependence: level(30),
            resentment: level(0),
            debt: level(15),
        }
    }

    fn fixture() -> Fixture {
        fixture_with_counsel_institution(OrganizationKind::LegalServices)
    }

    fn fixture_with_counsel_institution(counsel_kind: OrganizationKind) -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0x1A77_0A93);
        let sponsor = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "North Ward Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal sponsor should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "North Ward Police".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police authority should validate");
        let firm = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Marchetti & Vale".to_owned(),
                kind: counsel_kind,
            },
        )
        .expect("counsel institution should validate");
        let handler = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Legal Liaison".to_owned(),
                organization: Some(sponsor),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("contact handler should validate");
        let defendant = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Arrested Associate".to_owned(),
                organization: Some(sponsor),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("defendant should validate");
        let counsel = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Eleanor Vale".to_owned(),
                organization: Some(firm),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(88))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("counsel should validate");
        validate_set_relationship(&state, handler, counsel, relationship())
            .expect("lawyer relationship should validate")
            .commit(&mut state);
        let contact = validate_establish_contact(
            &state,
            InstitutionalContactDraft {
                sponsor,
                handler,
                contact: counsel,
            },
        )
        .expect("legal contact should validate")
        .commit(&mut state)
        .expect("legal contact should commit");

        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "North Ward conspiracy inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(defendant)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        let evidence = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(defendant),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("case evidence should validate")
        .commit(&mut state)
        .expect("case evidence should commit");
        let arrest = validate_arrest(
            &state,
            ArrestDraft {
                character: defendant,
                investigation,
                evidence: BTreeSet::from([evidence]),
            },
        )
        .expect("arrest should validate")
        .commit(&mut state)
        .expect("arrest should commit");

        let payer = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(sponsor),
                kind: AccountKind::AccountedFunds,
                label: "Legal reserve".to_owned(),
            },
        )
        .expect("payer account should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(sponsor),
                kind: AccountKind::Settlement,
                label: "Opening settlement".to_owned(),
            },
        )
        .expect("settlement account should validate");
        let provider = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(firm),
                kind: AccountKind::LegitimateOperating,
                label: "Client trust receipts".to_owned(),
            },
        )
        .expect("provider account should validate");
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Opening legal reserve".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-50_000),
                    },
                    LedgerPosting {
                        account: payer,
                        amount: Money::from_cents(50_000),
                    },
                ],
                authorization: None,
            },
        )
        .expect("opening reserve should validate")
        .commit(&mut state)
        .expect("opening reserve should commit");

        Fixture {
            registry,
            state,
            sponsor,
            handler,
            defendant,
            firm,
            counsel,
            contact,
            arrest,
            payer,
            provider,
        }
    }

    fn representation_draft(
        fixture: &Fixture,
        fee_cents: i64,
        authorization: Option<MandateAuthority>,
    ) -> LegalRepresentationDraft {
        LegalRepresentationDraft {
            arrest: fixture.arrest,
            sponsor: fixture.sponsor,
            contact: fixture.contact,
            fee: Money::from_cents(fee_cents),
            payer_account: fixture.payer,
            provider_account: fixture.provider,
            authorization,
        }
    }

    fn retain(
        fixture: &mut Fixture,
        fee_cents: i64,
        authorization: Option<MandateAuthority>,
    ) -> LegalRepresentationId {
        validate_retain_legal_representation(
            &fixture.state,
            representation_draft(fixture, fee_cents, authorization),
        )
        .expect("legal representation should validate")
        .commit(&mut fixture.state)
        .expect("legal representation should commit")
    }

    #[test]
    fn retained_counsel_is_paid_indexed_reported_and_survives_save() {
        let mut fixture = fixture();
        let representation = retain(&mut fixture, 12_000, None);
        let record = fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("representation should persist");
        assert_eq!(record.status(), LegalRepresentationStatus::Active);
        assert_eq!(record.defendant(), fixture.defendant);
        assert_eq!(record.counsel(), fixture.counsel);
        assert_eq!(record.counsel_institution(), fixture.firm);
        assert_eq!(record.contact(), fixture.contact);
        assert_eq!(record.fee(), Money::from_cents(12_000));
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.payer)
                .expect("payer should exist")
                .balance(),
            Money::from_cents(38_000)
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.provider)
                .expect("provider should exist")
                .balance(),
            Money::from_cents(12_000)
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .active_representation_for_arrest(fixture.arrest)
                .map(|record| record.id()),
            Some(representation)
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .representations_for_defendant(fixture.defendant)
                .count(),
            1
        );
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(record.report())
                .expect("retainer report should persist")
                .kind(),
            ReportKind::Legal
        );
        validate_state(&fixture.state).expect("retained-counsel state should validate");
        validate_invariants(&fixture.state);

        let save = build_save(&fixture.registry, &fixture.state)
            .expect("retained-counsel state should build a save");
        let bytes = bincode::serialize(&save).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let mut restored = restore_save(&fixture.registry, decoded)
            .expect("retained-counsel state should restore");
        assert_eq!(
            restored
                .legal()
                .active_representation_for_arrest(fixture.arrest)
                .map(|record| record.id()),
            Some(representation)
        );

        validate_end_legal_representation(
            &restored,
            representation,
            LegalRepresentationEndReason::MatterConcluded,
        )
        .expect("restored representation should be endable")
        .commit(&mut restored)
        .expect("representation end should commit");
        let ended = restored
            .legal()
            .get_legal_representation(representation)
            .expect("ended representation should remain historical");
        assert_eq!(ended.status(), LegalRepresentationStatus::Ended);
        assert_eq!(
            ended.end_reason(),
            Some(LegalRepresentationEndReason::MatterConcluded)
        );
        assert!(ended.ended_information().is_some());
        assert!(ended.ended_report().is_some());
        assert!(restored
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none());

        let replacement = validate_retain_legal_representation(
            &restored,
            LegalRepresentationDraft {
                arrest: fixture.arrest,
                sponsor: fixture.sponsor,
                contact: fixture.contact,
                fee: Money::from_cents(5_000),
                payer_account: fixture.payer,
                provider_account: fixture.provider,
                authorization: None,
            },
        )
        .expect("ended representation should permit later counsel retention")
        .commit(&mut restored)
        .expect("later representation should commit with fresh ID");
        assert_ne!(replacement, representation);
        assert_eq!(
            restored
                .legal()
                .representations_for_arrest(fixture.arrest)
                .count(),
            2
        );
        validate_state(&restored).expect("restored replacement-counsel state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn active_representation_locks_contact_until_representation_ends() {
        let mut fixture = fixture();
        let representation = retain(&mut fixture, 8_000, None);
        let error = validate_terminate_contact(&fixture.state, fixture.contact)
            .expect_err("active representation must retain its contact dependency");
        assert_eq!(
            error,
            ContactError::ActiveLegalRepresentation {
                contact: fixture.contact,
                representation,
            }
        );

        validate_end_legal_representation(
            &fixture.state,
            representation,
            LegalRepresentationEndReason::SponsorWithdrawn,
        )
        .expect("representation end should validate")
        .commit(&mut fixture.state)
        .expect("representation end should commit");
        validate_terminate_contact(&fixture.state, fixture.contact)
            .expect("ended representation should release contact dependency")
            .commit(&mut fixture.state)
            .expect("contact termination should commit");
        validate_state(&fixture.state).expect("ended representation history should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn active_representation_survives_defendant_departure_after_release() {
        let mut fixture = fixture();
        let representation = retain(&mut fixture, 7_500, None);
        validate_release_arrest(&fixture.state, fixture.arrest)
            .expect("defendant detention should release")
            .commit(&mut fixture.state)
            .expect("defendant release should commit");
        validate_reassign_character(&fixture.state, fixture.defendant, None, None)
            .expect("released defendant may leave sponsoring organization")
            .commit(&mut fixture.state)
            .expect("defendant departure should commit");
        let record = fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("representation should persist after defendant departure");
        assert_eq!(record.status(), LegalRepresentationStatus::Active);
        assert_eq!(record.sponsor(), fixture.sponsor);
        assert_eq!(record.defendant(), fixture.defendant);
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.defendant)
                .expect("defendant should persist")
                .organization(),
            None
        );
        validate_state(&fixture.state)
            .expect("active representation should survive post-release membership change");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn stale_retainer_after_contact_termination_is_atomic() {
        let mut fixture = fixture();
        let validated = validate_retain_legal_representation(
            &fixture.state,
            representation_draft(&fixture, 9_000, None),
        )
        .expect("initial retainer should validate");
        validate_terminate_contact(&fixture.state, fixture.contact)
            .expect("unused contact should terminate")
            .commit(&mut fixture.state)
            .expect("contact termination should commit");

        let error = validated
            .commit(&mut fixture.state)
            .expect_err("terminated contact must stale older retainer validation");
        assert!(matches!(
            error,
            LegalRepresentationError::StaleContact { .. }
        ));
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.payer)
                .expect("payer should exist")
                .balance(),
            Money::from_cents(50_000)
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.provider)
                .expect("provider should exist")
                .balance(),
            Money::ZERO
        );
        assert!(fixture
            .state
            .legal()
            .active_representation_for_arrest(fixture.arrest)
            .is_none());
        validate_state(&fixture.state)
            .expect("rejected stale retainer should preserve valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn delegated_legal_budget_authority_is_persisted_and_enforced() {
        let mut fixture = fixture();
        let mandate = validate_assign_mandate(
            &fixture.registry,
            &fixture.state,
            MandateDraft {
                organization: fixture.sponsor,
                manager: fixture.handler,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Legal,
                )]),
                standing_orders: BTreeMap::new(),
                budget: Some(BudgetAuthority {
                    funding_account: fixture.payer,
                    limit: Money::from_cents(15_000),
                    period: BudgetPeriod::Weekly,
                }),
            },
        )
        .expect("legal-support mandate should validate")
        .commit(&mut fixture.state)
        .expect("legal-support mandate should commit");
        let wrong_scope = MandateAuthority {
            mandate,
            manager: fixture.handler,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
        };
        let error = match validate_retain_legal_representation(
            &fixture.state,
            representation_draft(&fixture, 10_000, Some(wrong_scope)),
        ) {
            Ok(_) => panic!("non-Legal delegated scope must not authorize counsel retention"),
            Err(error) => error,
        };
        assert_eq!(error, LegalRepresentationError::InvalidAuthorityScope);

        let authority = MandateAuthority {
            mandate,
            manager: fixture.handler,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Legal),
        };
        let representation = retain(&mut fixture, 10_000, Some(authority));
        let record = fixture
            .state
            .legal()
            .get_legal_representation(representation)
            .expect("delegated representation should persist");
        assert_eq!(record.authorization(), Some(authority));
        let usage = fixture
            .state
            .finance()
            .get_transaction(record.payment())
            .expect("retainer payment should persist")
            .budget_usage()
            .expect("delegated retainer should persist budget usage");
        assert_eq!(usage.mandate(), mandate);
        assert_eq!(usage.manager(), fixture.handler);
        assert_eq!(usage.scope(), authority.scope);
        assert_eq!(usage.funding_account(), fixture.payer);
        assert_eq!(usage.amount(), Money::from_cents(10_000));
        validate_state(&fixture.state).expect("delegated legal-support state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn public_legal_authority_cannot_masquerade_as_private_defense_provider() {
        let fixture = fixture_with_counsel_institution(OrganizationKind::LegalAuthority);
        let error = match validate_retain_legal_representation(
            &fixture.state,
            representation_draft(&fixture, 6_000, None),
        ) {
            Ok(_) => {
                panic!("public legal authority contact must not become private defense counsel")
            }
            Err(error) => error,
        };
        assert_eq!(
            error,
            LegalRepresentationError::InvalidCounselInstitution(fixture.firm)
        );
        validate_state(&fixture.state).expect("rejected defense-provider state should validate");
        validate_invariants(&fixture.state);
    }
}
