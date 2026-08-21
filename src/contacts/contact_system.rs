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
use crate::intelligence::{InformationSourceKind, KnowledgeHolder};
use crate::social::{RelationshipDimensions, RelationshipRecord};
use crate::world::{Lifecycle, OrganizationKind};
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
    #[error("institutional contact character {0} is not active")]
    InactiveContact(CharacterId),
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
        .map(snapshot_relationship);
    let contact_to_handler = state
        .social
        .get_relationship(draft.contact, draft.handler)
        .filter(|relationship| has_relationship_basis(relationship.dimensions()))
        .map(snapshot_relationship);
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

pub const fn information_source_kind(kind: ContactKind) -> InformationSourceKind {
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

pub(crate) fn expected_contact_kind(kind: OrganizationKind) -> Option<ContactKind> {
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
    if sponsor_record.lifecycle() != Lifecycle::Active
        || sponsor_record.kind() != OrganizationKind::Criminal
    {
        return Err(ContactError::InvalidSponsor(sponsor));
    }
    let handler_record = state
        .world
        .get_character(handler)
        .ok_or(ContactError::MissingHandler(handler))?;
    if handler_record.lifecycle() != Lifecycle::Active
        || handler_record.organization() != Some(sponsor)
    {
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
    if contact_record.lifecycle() != Lifecycle::Active {
        return Err(ContactError::InactiveContact(contact));
    }
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
    if institution_record.lifecycle() != Lifecycle::Active {
        return Err(ContactError::InvalidInstitution(institution));
    }
    expected_contact_kind(institution_record.kind())
        .ok_or(ContactError::CriminalInstitution(institution))
}

fn validate_disclosure_source(
    state: &AppState,
    contact: &InstitutionalContactRecord,
    source: InformationId,
) -> Result<(), ContactError> {
    if let Some(arrest) = state.legal.active_arrest_for_character(contact.handler()) {
        return Err(ContactError::DetainedHandler {
            handler: contact.handler(),
            arrest: arrest.id(),
        });
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(contact.contact()) {
        return Err(ContactError::DetainedContact {
            contact: contact.contact(),
            arrest: arrest.id(),
        });
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
    Ok(())
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

fn snapshot_relationship(record: &RelationshipRecord) -> ContactRelationshipSnapshot {
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
    if state.now() == expected {
        Ok(())
    } else {
        Err(ContactError::StaleTime {
            expected,
            found: state.now(),
        })
    }
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
        Specificity,
    };
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    struct ContactFixture {
        registry: crate::registry::Registry,
        state: AppState,
        sponsor: OrganizationId,
        handler: CharacterId,
        institution: OrganizationId,
        source: CharacterId,
    }

    fn level(value: u8) -> RelationshipLevel {
        RelationshipLevel::try_new(value).expect("fixture relationship level should validate")
    }

    fn relationship(trust: u8, debt: u8) -> RelationshipDimensions {
        RelationshipDimensions {
            trust: level(trust),
            respect: level(35),
            fear: level(0),
            affection: level(15),
            dependence: level(20),
            resentment: level(0),
            debt: level(debt),
        }
    }

    fn make_fixture(institution_kind: OrganizationKind) -> ContactFixture {
        let registry = build_registry();
        let mut state = AppState::new(0x0C01_7AC7);
        let sponsor = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Contact Test Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("sponsor should validate");
        let institution = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Contact Test Institution".to_owned(),
                kind: institution_kind,
            },
        )
        .expect("institution should validate");
        let handler = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Contact Handler".to_owned(),
                organization: Some(sponsor),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("handler should validate");
        let source = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Institutional Source".to_owned(),
                organization: Some(institution),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("source should validate");
        validate_set_relationship(&state, handler, source, relationship(70, 45))
            .expect("contact relationship should validate")
            .commit(&mut state);
        ContactFixture {
            registry,
            state,
            sponsor,
            handler,
            institution,
            source,
        }
    }

    fn establish(fixture: &mut ContactFixture) -> ContactId {
        validate_establish_contact(
            &fixture.state,
            InstitutionalContactDraft {
                sponsor: fixture.sponsor,
                handler: fixture.handler,
                contact: fixture.source,
            },
        )
        .expect("institutional contact should validate")
        .commit(&mut fixture.state)
        .expect("institutional contact should commit")
    }

    fn record_source_information(
        fixture: &mut ContactFixture,
        holder: KnowledgeHolder,
    ) -> InformationId {
        validate_record_information(
            &fixture.state,
            InformationDraft {
                holder,
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::PoliceActivity,
                source_entity: None,
                subject: EntityRef::Character(fixture.handler),
                observed_at: fixture.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "Detectives have been asking questions about the contact handler."
                    .to_owned(),
            },
        )
        .expect("source information should validate")
        .commit(&mut fixture.state)
        .expect("source information should commit")
    }

    #[test]
    fn police_contact_disclosure_preserves_personal_source_provenance_and_save_round_trip() {
        let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
        let contact = establish(&mut fixture);
        assert_eq!(
            fixture
                .state
                .contacts()
                .get_contact(contact)
                .expect("contact should persist")
                .kind(),
            ContactKind::Police
        );
        let source_character = fixture.source;
        let source =
            record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
        let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
            .expect("personally held police information should be disclosable")
            .commit(&mut fixture.state)
            .expect("contact disclosure should commit");
        let disclosure_record = fixture
            .state
            .contacts()
            .get_disclosure(disclosure)
            .expect("disclosure should persist");
        let disclosed = fixture
            .state
            .intelligence()
            .get_information(disclosure_record.disclosed_information())
            .expect("disclosed information should persist");
        assert_eq!(
            disclosed.holder(),
            KnowledgeHolder::Organization(fixture.sponsor)
        );
        assert_eq!(
            disclosed.source_kind(),
            InformationSourceKind::PoliceContact
        );
        assert_eq!(
            disclosed.source_entity(),
            Some(EntityRef::Character(fixture.source))
        );
        assert_eq!(disclosed.derived_from(), &BTreeSet::from([source]));
        assert_eq!(disclosed.reliability(), Reliability::GenerallyReliable);
        assert_eq!(disclosed.specificity(), Specificity::Specific);
        assert_eq!(
            fixture
                .state
                .contacts()
                .disclosure_for_information(disclosed.id())
                .map(ContactDisclosureRecord::id),
            Some(disclosure)
        );
        validate_state(&fixture.state).expect("contact disclosure state should validate");
        validate_invariants(&fixture.state);

        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("contact disclosure state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&fixture.registry, decoded)
            .expect("contact disclosure state should restore");
        assert_eq!(
            restored
                .contacts()
                .get_disclosure(disclosure)
                .map(ContactDisclosureRecord::disclosed_information),
            Some(disclosed.id())
        );
        validate_invariants(&restored);
    }

    #[test]
    fn contact_disclosure_cannot_read_institution_owned_hidden_information() {
        let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
        let contact = establish(&mut fixture);
        let institution = fixture.institution;
        let hidden =
            record_source_information(&mut fixture, KnowledgeHolder::Organization(institution));
        let error = validate_contact_disclosure(&fixture.state, contact, hidden)
            .err()
            .expect("institution-owned truth must not pass through a personal contact implicitly");
        assert_eq!(
            error,
            ContactError::InformationUnavailable {
                information: hidden,
                contact: fixture.source,
            }
        );
        assert_eq!(
            fixture
                .state
                .contacts()
                .disclosures_for_contact(contact)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn active_contact_locks_memberships_until_termination_then_history_survives_moves() {
        let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
        let contact = establish(&mut fixture);
        let second_sponsor = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Second Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("second sponsor should validate");
        let second_institution = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Ward Office".to_owned(),
                kind: OrganizationKind::Political,
            },
        )
        .expect("second institution should validate");

        let handler_error = validate_reassign_character(
            &fixture.state,
            fixture.handler,
            Some(second_sponsor),
            None,
        )
        .expect_err("active contact handler must not leave sponsor");
        assert_eq!(
            handler_error,
            WorldError::ActiveInstitutionalContactHandler {
                character: fixture.handler,
                contact,
            }
        );
        let source_error = validate_reassign_character(
            &fixture.state,
            fixture.source,
            Some(second_institution),
            None,
        )
        .expect_err("active external contact must not leave institution");
        assert_eq!(
            source_error,
            WorldError::ActiveInstitutionalContactAssignment {
                character: fixture.source,
                contact,
            }
        );

        let source_character = fixture.source;
        let source =
            record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
        let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
            .expect("active contact disclosure should validate")
            .commit(&mut fixture.state)
            .expect("active contact disclosure should commit");
        validate_terminate_contact(&fixture.state, contact)
            .expect("active contact should terminate")
            .commit(&mut fixture.state)
            .expect("contact termination should commit");
        validate_reassign_character(&fixture.state, fixture.handler, Some(second_sponsor), None)
            .expect("terminated contact should release handler membership dependency")
            .commit(&mut fixture.state)
            .expect("handler move should commit");
        validate_reassign_character(
            &fixture.state,
            fixture.source,
            Some(second_institution),
            None,
        )
        .expect("terminated contact should release external membership dependency")
        .commit(&mut fixture.state)
        .expect("external contact move should commit");

        let historical = fixture
            .state
            .contacts()
            .get_contact(contact)
            .expect("terminated contact should remain historical");
        assert_eq!(historical.status(), ContactStatus::Terminated);
        assert_eq!(historical.institution(), fixture.institution);
        assert!(fixture
            .state
            .contacts()
            .get_disclosure(disclosure)
            .is_some());
        validate_state(&fixture.state)
            .expect("terminated contact history should survive personnel moves");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn establishment_token_rejects_relationship_change_without_partial_contact() {
        let mut fixture = make_fixture(OrganizationKind::Political);
        let stale = validate_establish_contact(
            &fixture.state,
            InstitutionalContactDraft {
                sponsor: fixture.sponsor,
                handler: fixture.handler,
                contact: fixture.source,
            },
        )
        .expect("contact establishment should initially validate");
        validate_set_relationship(
            &fixture.state,
            fixture.handler,
            fixture.source,
            relationship(40, 80),
        )
        .expect("relationship revision should validate")
        .commit(&mut fixture.state);
        let error = stale
            .commit(&mut fixture.state)
            .expect_err("relationship revision must stale establishment token");
        assert_eq!(
            error,
            ContactError::StaleRelationship {
                from: fixture.handler,
                to: fixture.source,
            }
        );
        assert_eq!(
            fixture
                .state
                .contacts()
                .contacts_for_sponsor(fixture.sponsor)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn disclosure_token_rejects_contact_termination_and_duplicate_source() {
        let mut fixture = make_fixture(OrganizationKind::Press);
        let contact = establish(&mut fixture);
        let source_character = fixture.source;
        let source =
            record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
        let stale = validate_contact_disclosure(&fixture.state, contact, source)
            .expect("contact disclosure should initially validate");
        validate_terminate_contact(&fixture.state, contact)
            .expect("contact termination should validate")
            .commit(&mut fixture.state)
            .expect("contact termination should commit");
        let error = stale
            .commit(&mut fixture.state)
            .expect_err("terminated contact must stale pending disclosure");
        assert_eq!(
            error,
            ContactError::StaleContact {
                contact,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            fixture
                .state
                .contacts()
                .disclosures_for_contact(contact)
                .count(),
            0
        );

        let mut duplicate_fixture = make_fixture(OrganizationKind::Press);
        let duplicate_contact = establish(&mut duplicate_fixture);
        let duplicate_source_character = duplicate_fixture.source;
        let duplicate_source = record_source_information(
            &mut duplicate_fixture,
            KnowledgeHolder::Character(duplicate_source_character),
        );
        validate_contact_disclosure(
            &duplicate_fixture.state,
            duplicate_contact,
            duplicate_source,
        )
        .expect("first disclosure should validate")
        .commit(&mut duplicate_fixture.state)
        .expect("first disclosure should commit");
        let duplicate_error = validate_contact_disclosure(
            &duplicate_fixture.state,
            duplicate_contact,
            duplicate_source,
        )
        .err()
        .expect("same source information must not be disclosed twice through one contact");
        assert_eq!(
            duplicate_error,
            ContactError::DuplicateDisclosure {
                contact: duplicate_contact,
                information: duplicate_source,
            }
        );
        validate_invariants(&fixture.state);
        validate_invariants(&duplicate_fixture.state);
    }

    #[test]
    fn institution_kind_controls_disclosure_channel_without_generic_influence_score() {
        for (organization_kind, contact_kind, source_kind) in [
            (
                OrganizationKind::LegalAuthority,
                ContactKind::Legal,
                InformationSourceKind::Lawyer,
            ),
            (
                OrganizationKind::Political,
                ContactKind::Political,
                InformationSourceKind::PoliticalContact,
            ),
            (
                OrganizationKind::Labor,
                ContactKind::Labor,
                InformationSourceKind::ProfessionalContact,
            ),
            (
                OrganizationKind::Commercial,
                ContactKind::Professional,
                InformationSourceKind::ProfessionalContact,
            ),
        ] {
            let mut fixture = make_fixture(organization_kind);
            let contact = establish(&mut fixture);
            assert_eq!(
                fixture
                    .state
                    .contacts()
                    .get_contact(contact)
                    .expect("contact should persist")
                    .kind(),
                contact_kind
            );
            let source_character = fixture.source;
            let source = record_source_information(
                &mut fixture,
                KnowledgeHolder::Character(source_character),
            );
            let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
                .expect("institutional disclosure should validate")
                .commit(&mut fixture.state)
                .expect("institutional disclosure should commit");
            let information = fixture
                .state
                .contacts()
                .get_disclosure(disclosure)
                .and_then(|record| {
                    fixture
                        .state
                        .intelligence()
                        .get_information(record.disclosed_information())
                })
                .expect("disclosed information should persist");
            assert_eq!(information.source_kind(), source_kind);
            validate_state(&fixture.state)
                .expect("typed institutional contact state should validate");
            validate_invariants(&fixture.state);
        }
    }
}
