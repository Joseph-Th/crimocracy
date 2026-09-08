//! Release-safe structural validation for the world, social, and contact subsystems.

use crate::contacts::contact_system::{
    information_source_kind, resolve_contact_kind_for_institution_kind,
};
use crate::contacts::{
    ContactDisclosureRecord, ContactRelationshipSnapshot, ContactStatus, InstitutionalContactRecord,
};
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::CharacterId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{InformationRecord, InformationSourceKind, KnowledgeHolder};
use crate::social::RelationshipRecord;
use crate::world::{
    ALL_POLICY_KINDS, BusinessOwner, BusinessRecord, CharacterRecord, OrganizationKind,
    OrganizationRecord,
};
use std::collections::BTreeSet;

pub(super) fn validate_world_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_player_organization(state)?;
    for organization in state.world.organizations() {
        validate_organization(organization)?;
    }
    for neighborhood in state.world.neighborhoods() {
        if neighborhood.name().trim().is_empty() {
            return Err(StateValidationError::EmptyEntityName {
                entity: EntityRef::Neighborhood(neighborhood.id()),
            });
        }
    }

    // One reused visitation set serves every character's ancestor walk; clearing between
    // characters keeps the cycle detection identical without allocating per record.
    let mut visited = BTreeSet::new();
    for character in state.world.characters() {
        validate_character(state, character, &mut visited)?;
    }
    for business in state.world.businesses() {
        validate_business(state, business)?;
    }
    Ok(())
}

fn validate_player_organization(state: &AppState) -> Result<(), StateValidationError> {
    let Some(player) = state.player_organization() else {
        return Ok(());
    };
    let organization =
        state
            .world
            .get_organization(player)
            .ok_or(StateValidationError::MissingEntity {
                context: "player organization",
                entity: EntityRef::Organization(player),
            })?;
    if organization.kind() != OrganizationKind::Criminal {
        return Err(StateValidationError::InvalidPlayerOrganization {
            organization: player,
        });
    }
    Ok(())
}

fn validate_organization(organization: &OrganizationRecord) -> Result<(), StateValidationError> {
    if organization.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Organization(organization.id()),
        });
    }
    for policy in ALL_POLICY_KINDS {
        let setting = organization
            .policy(policy)
            .ok_or(StateValidationError::MissingPolicy {
                organization: organization.id(),
                policy,
            })?;
        if setting.kind() != policy {
            return Err(StateValidationError::PolicyKindMismatch {
                organization: organization.id(),
                expected: policy,
                actual: setting.kind(),
            });
        }
    }
    Ok(())
}

fn validate_character(
    state: &AppState,
    character: &CharacterRecord,
    visited: &mut BTreeSet<CharacterId>,
) -> Result<(), StateValidationError> {
    if character.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Character(character.id()),
        });
    }
    if character.version() == 0 {
        return Err(StateValidationError::InvalidCharacterVersion {
            character: character.id(),
        });
    }
    if let Some(organization) = character.organization()
        && state.world.get_organization(organization).is_none()
    {
        return Err(StateValidationError::MissingEntity {
            context: "character organization",
            entity: EntityRef::Organization(organization),
        });
    }
    if let Some(supervisor) = character.supervisor() {
        let supervisor_record =
            state
                .world
                .get_character(supervisor)
                .ok_or(StateValidationError::MissingEntity {
                    context: "character supervisor",
                    entity: EntityRef::Character(supervisor),
                })?;
        if supervisor_record.organization() != character.organization() {
            return Err(StateValidationError::SupervisorOrganizationMismatch {
                character: character.id(),
                supervisor,
            });
        }
    }
    validate_supervision_chain(state, character, visited)
}

fn validate_supervision_chain(
    state: &AppState,
    character: &CharacterRecord,
    visited: &mut BTreeSet<CharacterId>,
) -> Result<(), StateValidationError> {
    visited.clear();
    let mut cursor = character.supervisor();
    while let Some(current) = cursor {
        if current == character.id() || !visited.insert(current) {
            return Err(StateValidationError::SupervisionCycle {
                character: character.id(),
            });
        }
        cursor = state
            .world
            .get_character(current)
            .ok_or(StateValidationError::MissingEntity {
                context: "supervision hierarchy",
                entity: EntityRef::Character(current),
            })?
            .supervisor();
    }
    Ok(())
}

fn validate_business(
    state: &AppState,
    business: &BusinessRecord,
) -> Result<(), StateValidationError> {
    if business.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Business(business.id()),
        });
    }
    if state
        .world
        .get_neighborhood(business.neighborhood())
        .is_none()
    {
        return Err(StateValidationError::MissingEntity {
            context: "business neighborhood",
            entity: EntityRef::Neighborhood(business.neighborhood()),
        });
    }
    if let Some(entity) = business_owner_entity(business.owner())
        && !is_entity_present(state, entity)
    {
        return Err(StateValidationError::MissingEntity {
            context: "business owner",
            entity,
        });
    }
    if business.version() == 0
        || state
            .world
            .get_business_ownership_change_for_version(business.id(), business.version())
            .is_none_or(|change| change.new_owner() != business.owner())
    {
        return Err(invalid_business_history(business));
    }
    validate_business_history(state, business)
}

fn validate_business_history(
    state: &AppState,
    business: &BusinessRecord,
) -> Result<(), StateValidationError> {
    for change in state.world.business_ownership_history(business.id()) {
        if change.changed_at() > state.now() {
            return Err(invalid_business_history(business));
        }
        for historical_owner in [change.previous_owner(), Some(change.new_owner())]
            .into_iter()
            .flatten()
        {
            if business_owner_entity(historical_owner)
                .is_some_and(|entity| !is_entity_present(state, entity))
            {
                return Err(invalid_business_history(business));
            }
        }
    }
    Ok(())
}

fn business_owner_entity(owner: BusinessOwner) -> Option<EntityRef> {
    match owner {
        BusinessOwner::Independent => None,
        BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
        BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
    }
}

fn invalid_business_history(business: &BusinessRecord) -> StateValidationError {
    StateValidationError::InvalidBusinessOwnershipHistory {
        business: business.id(),
    }
}

pub(super) fn validate_social_and_intelligence(
    state: &AppState,
) -> Result<(), StateValidationError> {
    for relationship in state.social.relationships() {
        validate_relationship(state, relationship)?;
    }
    for information in state.intelligence.information() {
        validate_information(state, information)?;
    }
    Ok(())
}

fn validate_relationship(
    state: &AppState,
    relationship: &RelationshipRecord,
) -> Result<(), StateValidationError> {
    if relationship.from() == relationship.to() || relationship.version() == 0 {
        return Err(StateValidationError::InvalidRelationship {
            from: relationship.from(),
            to: relationship.to(),
        });
    }
    for (context, entity) in [
        (
            "relationship source",
            EntityRef::Character(relationship.from()),
        ),
        (
            "relationship target",
            EntityRef::Character(relationship.to()),
        ),
    ] {
        if !is_entity_present(state, entity) {
            return Err(StateValidationError::MissingEntity { context, entity });
        }
    }
    Ok(())
}

fn validate_information(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    validate_information_references(state, information)?;
    if information.observed_at() > information.recorded_at()
        || information.recorded_at() > state.now()
    {
        return Err(StateValidationError::InvalidInformationChronology {
            information: information.id(),
        });
    }
    if information.source_kind() == InformationSourceKind::InternalReport {
        validate_internal_report_provenance(state, information)?;
    } else if !information.derived_from().is_empty() {
        validate_contact_derived_provenance(state, information)?;
    }
    validate_information_lineage(state, information)
}

fn validate_information_references(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    if information.summary().trim().is_empty() {
        return Err(StateValidationError::EmptyInformationSummary {
            information: information.id(),
        });
    }
    match information.holder() {
        KnowledgeHolder::Character(id) if state.world.get_character(id).is_none() => {
            return Err(StateValidationError::MissingEntity {
                context: "information holder",
                entity: EntityRef::Character(id),
            });
        }
        KnowledgeHolder::Organization(id) if state.world.get_organization(id).is_none() => {
            return Err(StateValidationError::MissingEntity {
                context: "information holder",
                entity: EntityRef::Organization(id),
            });
        }
        KnowledgeHolder::Character(_) | KnowledgeHolder::Organization(_) => {}
    }
    if !is_entity_present(state, information.subject()) {
        return Err(StateValidationError::MissingEntity {
            context: "information subject",
            entity: information.subject(),
        });
    }
    if let Some(source) = information.source_entity()
        && !is_entity_present(state, source)
    {
        return Err(StateValidationError::MissingEntity {
            context: "information source",
            entity: source,
        });
    }
    Ok(())
}

fn validate_internal_report_provenance(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    if information.derived_from().len() != 1 || information.source_entity().is_none() {
        return Err(invalid_provenance(information, information.id()));
    }
    let source = *information
        .derived_from()
        .iter()
        .next()
        .expect("validated internal report must have one provenance record");
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or_else(|| invalid_provenance(information, source))?;
    if !derived_information_matches_source(information, source_record)
        || information.source_entity() != Some(source_record.holder().entity())
    {
        return Err(invalid_provenance(information, source));
    }
    Ok(())
}

fn validate_contact_derived_provenance(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    let source = information
        .derived_from()
        .iter()
        .next()
        .copied()
        .ok_or_else(|| invalid_provenance(information, information.id()))?;
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or_else(|| invalid_provenance(information, source))?;
    let valid_contact_kind = matches!(
        information.source_kind(),
        InformationSourceKind::PoliceContact
            | InformationSourceKind::Lawyer
            | InformationSourceKind::PoliticalContact
            | InformationSourceKind::ProfessionalContact
            | InformationSourceKind::Press
    );
    if !valid_contact_kind
        || information.derived_from().len() != 1
        || state
            .contacts
            .disclosure_for_information(information.id())
            .is_none()
        || information.source_entity() != Some(source_record.holder().entity())
        || !matches!(source_record.holder(), KnowledgeHolder::Character(_))
        || !derived_information_matches_source(information, source_record)
    {
        return Err(invalid_provenance(information, source));
    }
    Ok(())
}

fn derived_information_matches_source(
    information: &InformationRecord,
    source: &InformationRecord,
) -> bool {
    information.topic() == source.topic()
        && information.subject() == source.subject()
        && information.observed_at() == source.observed_at()
        && information.reliability() == source.reliability()
        && information.specificity() == source.specificity()
        && information.summary() == source.summary()
}

fn validate_information_lineage(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    for source in information.derived_from() {
        let source_record = state
            .intelligence
            .get_information(*source)
            .ok_or_else(|| invalid_provenance(information, *source))?;
        if *source >= information.id() || source_record.recorded_at() > information.recorded_at() {
            return Err(invalid_provenance(information, *source));
        }
    }
    Ok(())
}

fn invalid_provenance(
    information: &InformationRecord,
    source_information: crate::core::id::InformationId,
) -> StateValidationError {
    StateValidationError::InvalidInformationProvenance {
        information: information.id(),
        source_information,
    }
}

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
        || disclosed.reliability() != source.reliability()
        || disclosed.specificity() != source.specificity()
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
            && relationship_dimensions_have_basis(snapshot.dimensions())
    };
    let forward =
        handler_to_contact.is_some_and(|snapshot| valid_snapshot(snapshot, handler, contact));
    let reverse =
        contact_to_handler.is_some_and(|snapshot| valid_snapshot(snapshot, contact, handler));
    (forward || reverse)
        && handler_to_contact.is_none_or(|snapshot| valid_snapshot(snapshot, handler, contact))
        && contact_to_handler.is_none_or(|snapshot| valid_snapshot(snapshot, contact, handler))
}

fn relationship_dimensions_have_basis(dimensions: crate::social::RelationshipDimensions) -> bool {
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
