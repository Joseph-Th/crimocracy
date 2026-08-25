//! Canonical establishment, termination, and information disclosure for institutional contacts.

use crate::contacts::{
    ContactDisclosureRecord, ContactKind, ContactRelationshipSnapshot, ContactStatus,
    InstitutionalContactRecord,
};
use crate::core::id::{
    ArrestId, CharacterId, ContactDisclosureId, ContactId, IdExhaustionError, IdKind,
    InformationId, LegalRepresentationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::intelligence_system::{
    validate_contact_information_derivation, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{InformationSourceKind, InformationTopic, KnowledgeHolder};
use crate::social::{RelationshipDimensions, RelationshipRecord};
use crate::world::OrganizationKind;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ContactError {
    #[error("sponsor organization {0} does not exist")]
    MissingSponsor(OrganizationId),
    #[error("organization {0} is not an active criminal organization")]
    InvalidSponsor(OrganizationId),
    #[error("contact handler {0} does not exist")]
    MissingHandler(CharacterId),
    #[error("contact handler {handler} is not an active member of sponsor {sponsor}")]
    InvalidHandler {
        handler: CharacterId,
        sponsor: OrganizationId,
    },
    #[error("contact handler {handler} is detained under arrest {arrest}")]
    DetainedHandler {
        handler: CharacterId,
        arrest: ArrestId,
    },
    #[error("institutional contact character {contact} is detained under arrest {arrest}")]
    DetainedContact {
        contact: CharacterId,
        arrest: ArrestId,
    },
    #[error("institutional contact character {0} does not exist")]
    MissingContact(CharacterId),
    #[error("institutional contact character {0} has no institution")]
    ContactHasNoInstitution(CharacterId),
    #[error("institution {0} does not exist or is inactive")]
    InvalidInstitution(OrganizationId),
    #[error("criminal organization {0} cannot be used as an institutional contact source")]
    CriminalInstitution(OrganizationId),
    #[error("handler and institutional contact must be different characters")]
    SelfContact,
    #[error("handler {handler} and contact {contact} have no established social relationship")]
    NoRelationship {
        handler: CharacterId,
        contact: CharacterId,
    },
    #[error("sponsor {sponsor} already has active institutional contact {existing} with character {contact}")]
    DuplicateActiveContact {
        sponsor: OrganizationId,
        contact: CharacterId,
        existing: ContactId,
    },
    #[error("institutional contact {0} does not exist")]
    MissingContactRecord(ContactId),
    #[error("institutional contact {0} is not active")]
    ContactNotActive(ContactId),
    #[error(
        "institutional contact {contact} supports active legal representation {representation}"
    )]
    ActiveLegalRepresentation {
        contact: ContactId,
        representation: LegalRepresentationId,
    },
    #[error("institutional contact {contact} changed after validation; expected version {expected}, found {found}")]
    StaleContact {
        contact: ContactId,
        expected: u32,
        found: u32,
    },
    #[error("character {character} changed after contact validation; expected version {expected}, found {found}")]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("relationship from {from} to {to} changed after contact validation")]
    StaleRelationship { from: CharacterId, to: CharacterId },
    #[error(
        "contact transaction was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("information {information} is not personally held by contact character {contact}")]
    InformationUnavailable {
        information: InformationId,
        contact: CharacterId,
    },
    #[error(
        "information {information} topic {topic:?} is outside the domain of {kind:?} contacts"
    )]
    InformationOutsideContactDomain {
        information: InformationId,
        topic: crate::intelligence::InformationTopic,
        kind: ContactKind,
    },
    #[error("information {information} was already disclosed through contact {contact}")]
    DuplicateDisclosure {
        contact: ContactId,
        information: InformationId,
    },
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Debug)]
pub struct InstitutionalContactDraft {
    pub sponsor: OrganizationId,
    pub handler: CharacterId,
    pub contact: CharacterId,
}

#[derive(Debug)]
pub struct ValidatedContactEstablishment {
    draft: InstitutionalContactDraft,
    institution: OrganizationId,
    kind: ContactKind,
    handler_version: u32,
    contact_version: u32,
    handler_to_contact: Option<ContactRelationshipSnapshot>,
    contact_to_handler: Option<ContactRelationshipSnapshot>,
    validated_at: SimTime,
}

impl ValidatedContactEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<ContactId, ContactError> {
        validate_time(state, self.validated_at)?;
        validate_character_version(state, self.draft.handler, self.handler_version)?;
        validate_character_version(state, self.draft.contact, self.contact_version)?;
        validate_contact_dependencies(
            state,
            self.draft.sponsor,
            self.draft.handler,
            self.draft.contact,
        )?;
        validate_relationship_snapshot(state, self.handler_to_contact)?;
        validate_relationship_snapshot(state, self.contact_to_handler)?;
        ensure_no_active_duplicate(state, self.draft.sponsor, self.draft.contact)?;
        let contact_record = state
            .world
            .get_character(self.draft.contact)
            .expect("validated institutional contact must exist");
        // The contact's version was re-validated above and every membership change bumps the
        // character version, so the institution binding and derived contact kind are stable
        // between validation and commit; neither can legitimately differ here.
        debug_assert_eq!(
            contact_record.organization(),
            Some(self.institution),
            "validated contact lost its institution without a version change"
        );
        let id = state.ids.next_contact()?;
        state.contacts.insert_contact(InstitutionalContactRecord {
            id,
            parties: super::ContactParties {
                sponsor: self.draft.sponsor,
                handler: self.draft.handler,
                contact: self.draft.contact,
                institution: self.institution,
                kind: self.kind,
            },
            relationship_basis: super::ContactRelationshipBasis {
                handler_to_contact: self.handler_to_contact,
                contact_to_handler: self.contact_to_handler,
            },
            lifecycle: super::ContactLifecycle {
                status: ContactStatus::Active,
                established_at: self.validated_at,
                terminated_at: None,
                version: 1,
            },
        });
        Ok(id)
    }
}

pub fn validate_establish_contact(
    state: &AppState,
    draft: InstitutionalContactDraft,
) -> Result<ValidatedContactEstablishment, ContactError> {
    if draft.handler == draft.contact {
        return Err(ContactError::SelfContact);
    }
    validate_contact_dependencies(state, draft.sponsor, draft.handler, draft.contact)?;
    ensure_no_active_duplicate(state, draft.sponsor, draft.contact)?;
    let contact = state
        .world
        .get_character(draft.contact)
        .ok_or(ContactError::MissingContact(draft.contact))?;
    let institution = contact
        .organization()
        .ok_or(ContactError::ContactHasNoInstitution(draft.contact))?;
    let kind = resolve_contact_kind(state, institution)?;
    let handler = state
        .world
        .get_character(draft.handler)
        .ok_or(ContactError::MissingHandler(draft.handler))?;
    let handler_to_contact = state
        .social
        .get_relationship(draft.handler, draft.contact)
        .filter(|relationship| has_relationship_basis(relationship.dimensions()))
        .map(build_contact_relationship_snapshot);
    let contact_to_handler = state
        .social
        .get_relationship(draft.contact, draft.handler)
        .filter(|relationship| has_relationship_basis(relationship.dimensions()))
        .map(build_contact_relationship_snapshot);
    if handler_to_contact.is_none() && contact_to_handler.is_none() {
        return Err(ContactError::NoRelationship {
            handler: draft.handler,
            contact: draft.contact,
        });
    }
    Ok(ValidatedContactEstablishment {
        draft,
        institution,
        kind,
        handler_version: handler.version(),
        contact_version: contact.version(),
        handler_to_contact,
        contact_to_handler,
        validated_at: state.now(),
    })
}

#[derive(Debug)]
pub struct ValidatedContactTermination {
    contact: ContactId,
    expected_version: u32,
    validated_at: SimTime,
}

impl ValidatedContactTermination {
    pub fn commit(self, state: &mut AppState) -> Result<ContactId, ContactError> {
        validate_time(state, self.validated_at)?;
        let record = state
            .contacts
            .get_contact(self.contact)
            .ok_or(ContactError::MissingContactRecord(self.contact))?;
        if record.version() != self.expected_version {
            return Err(ContactError::StaleContact {
                contact: self.contact,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != ContactStatus::Active {
            return Err(ContactError::ContactNotActive(self.contact));
        }
        if let Some(representation) = state
            .legal
            .active_representations_for_contact(self.contact)
            .next()
        {
            return Err(ContactError::ActiveLegalRepresentation {
                contact: self.contact,
                representation: representation.id(),
            });
        }
        state
            .contacts
            .terminate_contact(self.contact, self.validated_at);
        Ok(self.contact)
    }
}

pub fn validate_terminate_contact(
    state: &AppState,
    contact: ContactId,
) -> Result<ValidatedContactTermination, ContactError> {
    let record = state
        .contacts
        .get_contact(contact)
        .ok_or(ContactError::MissingContactRecord(contact))?;
    if record.status() != ContactStatus::Active {
        return Err(ContactError::ContactNotActive(contact));
    }
    if let Some(representation) = state
        .legal
        .active_representations_for_contact(contact)
        .next()
    {
        return Err(ContactError::ActiveLegalRepresentation {
            contact,
            representation: representation.id(),
        });
    }
    Ok(ValidatedContactTermination {
        contact,
        expected_version: record.version(),
        validated_at: state.now(),
    })
}

pub struct ValidatedContactDisclosure {
    contact: ContactId,
    source: InformationId,
    expected_contact_version: u32,
    disclosed_at: SimTime,
    information: ValidatedInformation,
}

impl ValidatedContactDisclosure {
    pub fn commit(self, state: &mut AppState) -> Result<ContactDisclosureId, ContactError> {
        state
            .ids
            .reserve_many(&[(IdKind::Information, 1), (IdKind::ContactDisclosure, 1)])?;
        validate_time(state, self.disclosed_at)?;
        let record = state
            .contacts
            .get_contact(self.contact)
            .ok_or(ContactError::MissingContactRecord(self.contact))?;
        if record.version() != self.expected_contact_version {
            return Err(ContactError::StaleContact {
                contact: self.contact,
                expected: self.expected_contact_version,
                found: record.version(),
            });
        }
        if record.status() != ContactStatus::Active {
            return Err(ContactError::ContactNotActive(self.contact));
        }
        validate_disclosure_source(state, record, self.source)?;
        ensure_disclosure_not_duplicate(state, self.contact, self.source)?;
        let disclosed_information = self.information.commit(state)?;
        let id = state.ids.next_contact_disclosure()?;
        state.contacts.insert_disclosure(ContactDisclosureRecord {
            id,
            contact: self.contact,
            source_information: self.source,
            disclosed_information,
            disclosed_at: self.disclosed_at,
        });
        Ok(id)
    }
}

pub fn validate_contact_disclosure(
    state: &AppState,
    contact: ContactId,
    source: InformationId,
) -> Result<ValidatedContactDisclosure, ContactError> {
    let record = state
        .contacts
        .get_contact(contact)
        .ok_or(ContactError::MissingContactRecord(contact))?;
    if record.status() != ContactStatus::Active {
        return Err(ContactError::ContactNotActive(contact));
    }
    validate_disclosure_source(state, record, source)?;
    ensure_disclosure_not_duplicate(state, contact, source)?;
    let information = validate_contact_information_derivation(
        state,
        source,
        record.contact(),
        record.sponsor(),
        information_source_kind(record.kind()),
    )?;
    Ok(ValidatedContactDisclosure {
        contact,
        source,
        expected_contact_version: record.version(),
        disclosed_at: state.now(),
        information,
    })
}

/// Total contact-kind to information-source translation; a constant mapping, not a
/// state-resolving lookup.
pub(crate) const fn information_source_kind(kind: ContactKind) -> InformationSourceKind {
    match kind {
        ContactKind::Police => InformationSourceKind::PoliceContact,
        ContactKind::Legal => InformationSourceKind::Lawyer,
        ContactKind::Political => InformationSourceKind::PoliticalContact,
        ContactKind::Press => InformationSourceKind::Press,
        ContactKind::Labor | ContactKind::Professional => {
            InformationSourceKind::ProfessionalContact
        }
    }
}

pub(crate) fn resolve_contact_kind_for_institution_kind(
    kind: OrganizationKind,
) -> Option<ContactKind> {
    match kind {
        OrganizationKind::LawEnforcement => Some(ContactKind::Police),
        OrganizationKind::LegalAuthority
        | OrganizationKind::LegalServices
        | OrganizationKind::Prosecutor => Some(ContactKind::Legal),
        OrganizationKind::Political => Some(ContactKind::Political),
        OrganizationKind::Press => Some(ContactKind::Press),
        OrganizationKind::Labor => Some(ContactKind::Labor),
        OrganizationKind::Civic | OrganizationKind::Commercial => Some(ContactKind::Professional),
        OrganizationKind::Criminal => None,
    }
}

fn validate_contact_dependencies(
    state: &AppState,
    sponsor: OrganizationId,
    handler: CharacterId,
    contact: CharacterId,
) -> Result<(), ContactError> {
    let sponsor_record = state
        .world
        .get_organization(sponsor)
        .ok_or(ContactError::MissingSponsor(sponsor))?;
    if sponsor_record.kind() != OrganizationKind::Criminal {
        return Err(ContactError::InvalidSponsor(sponsor));
    }
    let handler_record = state
        .world
        .get_character(handler)
        .ok_or(ContactError::MissingHandler(handler))?;
    if handler_record.organization() != Some(sponsor) {
        return Err(ContactError::InvalidHandler { handler, sponsor });
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(handler) {
        return Err(ContactError::DetainedHandler {
            handler,
            arrest: arrest.id(),
        });
    }
    let contact_record = state
        .world
        .get_character(contact)
        .ok_or(ContactError::MissingContact(contact))?;
    // A detained external contact cannot serve as an institutional channel: custody blocks
    // contact handling for the person themselves, which calls must not treat as a working source.
    if let Some(arrest) = state.legal.active_arrest_for_character(contact) {
        return Err(ContactError::DetainedContact {
            contact,
            arrest: arrest.id(),
        });
    }
    let institution = contact_record
        .organization()
        .ok_or(ContactError::ContactHasNoInstitution(contact))?;
    resolve_contact_kind(state, institution)?;
    Ok(())
}

fn resolve_contact_kind(
    state: &AppState,
    institution: OrganizationId,
) -> Result<ContactKind, ContactError> {
    let institution_record = state
        .world
        .get_organization(institution)
        .ok_or(ContactError::InvalidInstitution(institution))?;
    resolve_contact_kind_for_institution_kind(institution_record.kind())
        .ok_or(ContactError::CriminalInstitution(institution))
}

/// Whether both human endpoints of the channel are out of custody. The pending-disclosure
/// offer surface and the disclosure commit gate share this rule so their answers can never
/// disagree.
/// Whether both live endpoints of the contact channel can still transact.
fn have_channel_endpoints_available(
    state: &AppState,
    contact: &InstitutionalContactRecord,
) -> bool {
    state
        .legal
        .active_arrest_for_character(contact.handler())
        .is_none()
        && state
            .legal
            .active_arrest_for_character(contact.contact())
            .is_none()
}

fn detention_error(state: &AppState, contact: &InstitutionalContactRecord) -> ContactError {
    if let Some(arrest) = state.legal.active_arrest_for_character(contact.handler()) {
        return ContactError::DetainedHandler {
            handler: contact.handler(),
            arrest: arrest.id(),
        };
    }
    match state.legal.active_arrest_for_character(contact.contact()) {
        Some(arrest) => ContactError::DetainedContact {
            contact: contact.contact(),
            arrest: arrest.id(),
        },
        // Unreachable through the shared predicate (it verified both endpoints), but the
        // handler-side error is the honest fallback if the two checks ever disagree.
        None => ContactError::MissingContact(contact.contact()),
    }
}

fn validate_disclosure_source(
    state: &AppState,
    contact: &InstitutionalContactRecord,
    source: InformationId,
) -> Result<(), ContactError> {
    // One shared availability rule backs both the offer surface and this commit gate, so a
    // detained handler or contact can never appear actionable and then fail here.
    if !have_channel_endpoints_available(state, contact) {
        return Err(detention_error(state, contact));
    }
    let information = state
        .intelligence
        .get_information(source)
        .ok_or(ContactError::MissingInformation(source))?;
    if information.holder() != KnowledgeHolder::Character(contact.contact()) {
        return Err(ContactError::InformationUnavailable {
            information: source,
            contact: contact.contact(),
        });
    }
    // A contact can only vouch for knowledge inside their institutional domain; this stops a
    // channel from laundering unrelated personal knowledge under institutional provenance.
    if !disclosable_topics(contact.kind()).contains(&information.topic()) {
        return Err(ContactError::InformationOutsideContactDomain {
            information: source,
            topic: information.topic(),
            kind: contact.kind(),
        });
    }
    Ok(())
}

/// The contact-channel "ask what he knows" surface: the information records the contact
/// personally holds, inside the channel's institutional domain, that this sponsor has not
/// already been told. Returning identities and nothing more keeps content behind the canonical
/// disclosure path; the caller chooses which topics to actually hear about.
pub fn find_pending_disclosure_sources(
    state: &crate::core::state::AppState,
    contact: ContactId,
) -> Vec<InformationId> {
    let Some(record) = state.contacts().get_contact(contact) else {
        return Vec::new();
    };
    if record.status() != ContactStatus::Active || !have_channel_endpoints_available(state, record)
    {
        return Vec::new();
    }
    let topics = disclosable_topics(record.kind());
    let mut sources = Vec::new();
    for topic in topics {
        for information in state
            .intelligence()
            .information_for_holder_by_topic(KnowledgeHolder::Character(record.contact()), *topic)
        {
            if state
                .contacts()
                .disclosure_from_source(contact, information.id())
                .is_none()
            {
                sources.push(information.id());
            }
        }
    }
    sources.sort_unstable();
    sources.dedup();
    sources
}

/// Topics each contact channel credibly knows through its institution.
fn disclosable_topics(kind: ContactKind) -> &'static [InformationTopic] {
    match kind {
        // Law enforcement gathers exactly the security and patrol picture it enforces.
        ContactKind::Police => &[
            InformationTopic::General,
            InformationTopic::PoliceActivity,
            InformationTopic::LegalActivity,
            InformationTopic::TargetSecurity,
            InformationTopic::Schedule,
            InformationTopic::Route,
        ],
        ContactKind::Legal => &[InformationTopic::General, InformationTopic::LegalActivity],
        ContactKind::Political => &[InformationTopic::General, InformationTopic::MarketAccess],
        ContactKind::Press => &[InformationTopic::General],
        ContactKind::Labor | ContactKind::Professional => &[
            InformationTopic::General,
            InformationTopic::FinancialPerformance,
            InformationTopic::Personnel,
        ],
    }
}

fn ensure_no_active_duplicate(
    state: &AppState,
    sponsor: OrganizationId,
    contact: CharacterId,
) -> Result<(), ContactError> {
    if let Some(existing) = state.contacts.active_contact_for(sponsor, contact) {
        return Err(ContactError::DuplicateActiveContact {
            sponsor,
            contact,
            existing: existing.id(),
        });
    }
    Ok(())
}

fn ensure_disclosure_not_duplicate(
    state: &AppState,
    contact: ContactId,
    source: InformationId,
) -> Result<(), ContactError> {
    if state
        .contacts
        .disclosure_from_source(contact, source)
        .is_some()
    {
        return Err(ContactError::DuplicateDisclosure {
            contact,
            information: source,
        });
    }
    Ok(())
}

fn build_contact_relationship_snapshot(record: &RelationshipRecord) -> ContactRelationshipSnapshot {
    ContactRelationshipSnapshot {
        from: record.from(),
        to: record.to(),
        dimensions: record.dimensions(),
        version: record.version(),
    }
}

fn validate_relationship_snapshot(
    state: &AppState,
    snapshot: Option<ContactRelationshipSnapshot>,
) -> Result<(), ContactError> {
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    if !state
        .social
        .get_relationship(snapshot.from(), snapshot.to())
        .is_some_and(|relationship| {
            relationship.version() == snapshot.version()
                && relationship.dimensions() == snapshot.dimensions()
        })
    {
        return Err(ContactError::StaleRelationship {
            from: snapshot.from(),
            to: snapshot.to(),
        });
    }
    Ok(())
}

fn validate_character_version(
    state: &AppState,
    character: CharacterId,
    expected: u32,
) -> Result<(), ContactError> {
    let record = state
        .world
        .get_character(character)
        .ok_or(ContactError::MissingContact(character))?;
    if record.version() != expected {
        return Err(ContactError::StaleCharacter {
            character,
            expected,
            found: record.version(),
        });
    }
    Ok(())
}

fn validate_time(state: &AppState, expected: SimTime) -> Result<(), ContactError> {
    crate::core::time::ensure_time_current(state.now(), expected)
        .map_err(|(expected, found)| ContactError::StaleTime { expected, found })
}

fn has_relationship_basis(dimensions: RelationshipDimensions) -> bool {
    [
        dimensions.trust,
        dimensions.respect,
        dimensions.fear,
        dimensions.affection,
        dimensions.dependence,
        dimensions.resentment,
        dimensions.debt,
    ]
    .into_iter()
    .any(|level| level.value() > 0)
}

#[cfg(test)]
mod tests;
