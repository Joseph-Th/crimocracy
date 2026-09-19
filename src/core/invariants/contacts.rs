//! Release-safe structural validation for institutional contacts and disclosures.

use crate::contacts::contact_system::{
    information_source_kind, resolve_contact_kind_for_institution_kind,
};
use crate::contacts::{
    ContactDisclosureRecord, ContactRelationshipSnapshot, ContactStatus, InstitutionalContactRecord,
};
use crate::core::entity::EntityRef;
use crate::core::id::CharacterId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{
    KnowledgeHolder, downgraded_reliability_for_contact_derivation,
    downgraded_specificity_for_contact_derivation,
};
use crate::world::{CharacterRecord, OrganizationKind};

pub(super) fn validate_contacts(state: &AppState) -> Result<(), StateValidationError> {
    for contact in state.contacts.contacts() {
        validate_contact(state, contact)?;
    }

    for disclosure in state.contacts.disclosures() {
        validate_contact_disclosure(state, disclosure)?;
    }
    Ok(())
}

fn validate_contact(
    state: &AppState,
    contact: &InstitutionalContactRecord,
) -> Result<(), StateValidationError> {
    let sponsor = state
        .world
        .get_organization(contact.sponsor())
        .ok_or_else(|| invalid_contact(contact))?;
    let handler = state
        .world
        .get_character(contact.handler())
        .ok_or_else(|| invalid_contact(contact))?;
    let source = state
        .world
        .get_character(contact.contact())
        .ok_or_else(|| invalid_contact(contact))?;
    let institution = state
        .world
        .get_organization(contact.institution())
        .ok_or_else(|| invalid_contact(contact))?;
    if sponsor.kind() != OrganizationKind::Criminal
        || resolve_contact_kind_for_institution_kind(institution.kind()) != Some(contact.kind())
        || contact.handler() == contact.contact()
        || contact.established_at() > state.now()
        || !contact_relationship_basis_is_valid(
            contact.handler(),
            contact.contact(),
            contact.handler_to_contact(),
            contact.contact_to_handler(),
        )
    {
        return Err(invalid_contact(contact));
    }
    match contact.status() {
        ContactStatus::Active => validate_active_contact(state, contact, handler, source),
        ContactStatus::Terminated => validate_terminated_contact(state, contact),
    }
}

fn validate_active_contact(
    state: &AppState,
    contact: &InstitutionalContactRecord,
    handler: &CharacterRecord,
    source: &CharacterRecord,
) -> Result<(), StateValidationError> {
    if contact.version() != 1
        || contact.terminated_at().is_some()
        || handler.organization() != Some(contact.sponsor())
        || source.organization() != Some(contact.institution())
        || state
            .contacts
            .active_contact_for(contact.sponsor(), contact.contact())
            .is_none_or(|current| current.id() != contact.id())
    {
        return Err(invalid_contact(contact));
    }
    Ok(())
}

fn validate_terminated_contact(
    state: &AppState,
    contact: &InstitutionalContactRecord,
) -> Result<(), StateValidationError> {
    let terminated_at = contact
        .terminated_at()
        .ok_or_else(|| invalid_contact(contact))?;
    if contact.version() != 2
        || terminated_at < contact.established_at()
        || terminated_at > state.now()
    {
        return Err(invalid_contact(contact));
    }
    Ok(())
}

fn validate_contact_disclosure(
    state: &AppState,
    disclosure: &ContactDisclosureRecord,
) -> Result<(), StateValidationError> {
    let contact = state
        .contacts
        .get_contact(disclosure.contact())
        .ok_or_else(|| invalid_disclosure(disclosure))?;
    let source = state
        .intelligence
        .get_information(disclosure.source_information())
        .ok_or_else(|| invalid_disclosure(disclosure))?;
    let disclosed = state
        .intelligence
        .get_information(disclosure.disclosed_information())
        .ok_or_else(|| invalid_disclosure(disclosure))?;
    if disclosure.disclosed_at() < contact.established_at()
        || disclosure.disclosed_at() > state.now()
        || contact
            .terminated_at()
            .is_some_and(|terminated_at| disclosure.disclosed_at() > terminated_at)
        || source.holder() != KnowledgeHolder::Character(contact.contact())
        || source.recorded_at() > disclosure.disclosed_at()
        || source.observed_at() > disclosure.disclosed_at()
        || disclosed.holder() != KnowledgeHolder::Organization(contact.sponsor())
        || disclosed.source_kind() != information_source_kind(contact.kind())
        || disclosed.source_entity() != Some(EntityRef::Character(contact.contact()))
        || disclosed.topic() != source.topic()
        || disclosed.subject() != source.subject()
        || disclosed.observed_at() != source.observed_at()
        || disclosed.recorded_at() != disclosure.disclosed_at()
        || disclosed.reliability()
            != downgraded_reliability_for_contact_derivation(source.reliability())
        || disclosed.specificity()
            != downgraded_specificity_for_contact_derivation(source.specificity())
        || disclosed.summary() != source.summary()
        || disclosed.derived_from().len() != 1
        || !disclosed.derived_from().contains(&source.id())
        || state
            .contacts
            .disclosure_for_information(disclosed.id())
            .is_none_or(|record| record.id() != disclosure.id())
    {
        return Err(invalid_disclosure(disclosure));
    }
    Ok(())
}

fn invalid_contact(contact: &InstitutionalContactRecord) -> StateValidationError {
    StateValidationError::InvalidInstitutionalContact {
        contact: contact.id(),
    }
}

fn invalid_disclosure(disclosure: &ContactDisclosureRecord) -> StateValidationError {
    StateValidationError::InvalidContactDisclosure {
        disclosure: disclosure.id(),
    }
}

fn contact_relationship_basis_is_valid(
    handler: CharacterId,
    contact: CharacterId,
    handler_to_contact: Option<ContactRelationshipSnapshot>,
    contact_to_handler: Option<ContactRelationshipSnapshot>,
) -> bool {
    let valid_snapshot = |snapshot: ContactRelationshipSnapshot, from, to| {
        snapshot.from() == from
            && snapshot.to() == to
            && snapshot.version() > 0
            && crate::contacts::contact_system::has_contact_relationship_basis(
                snapshot.dimensions(),
            )
    };
    let forward =
        handler_to_contact.is_some_and(|snapshot| valid_snapshot(snapshot, handler, contact));
    let reverse =
        contact_to_handler.is_some_and(|snapshot| valid_snapshot(snapshot, contact, handler));
    (forward || reverse)
        && handler_to_contact.is_none_or(|snapshot| valid_snapshot(snapshot, handler, contact))
        && contact_to_handler.is_none_or(|snapshot| valid_snapshot(snapshot, contact, handler))
}
