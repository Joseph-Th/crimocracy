//! Release-safe structural validation for the world, social, and contact subsystems.

use crate::contacts::contact_system::{expected_contact_kind, resolve_information_source_kind};
use crate::contacts::ContactRelationshipSnapshot;
use crate::contacts::ContactStatus;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::CharacterId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{InformationSourceKind, KnowledgeHolder};
use crate::world::{BusinessOwner, Lifecycle, OrganizationKind, ALL_POLICY_KINDS};
use std::collections::BTreeSet;

pub(super) fn validate_world_state(state: &AppState) -> Result<(), StateValidationError> {
    if let Some(player) = state.player_organization() {
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
    }

    for organization in state.world.organizations() {
        for policy in ALL_POLICY_KINDS {
            let setting =
                organization
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
    }

    for character in state.world.characters() {
        if let Some(organization) = character.organization() {
            if state.world.get_organization(organization).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "character organization",
                    entity: EntityRef::Organization(organization),
                });
            }
        }
        if let Some(supervisor) = character.supervisor() {
            let supervisor_record = state.world.get_character(supervisor).ok_or(
                StateValidationError::MissingEntity {
                    context: "character supervisor",
                    entity: EntityRef::Character(supervisor),
                },
            )?;
            if supervisor_record.organization() != character.organization() {
                return Err(StateValidationError::SupervisorOrganizationMismatch {
                    character: character.id(),
                    supervisor,
                });
            }
        }
        let mut visited = BTreeSet::new();
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
    }

    for business in state.world.businesses() {
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
        let owner = match business.owner() {
            BusinessOwner::Independent => None,
            BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
            BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
        };
        if let Some(entity) = owner {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "business owner",
                    entity,
                });
            }
        }
        if business.version() == 0
            || state
                .world
                .get_business_ownership_change_for_version(business.id(), business.version())
                .is_none_or(|change| change.new_owner() != business.owner())
        {
            return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                business: business.id(),
            });
        }
        for change in state.world.business_ownership_history(business.id()) {
            if change.changed_at() > state.now() {
                return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                    business: business.id(),
                });
            }
            for historical_owner in [change.previous_owner(), Some(change.new_owner())]
                .into_iter()
                .flatten()
            {
                let entity = match historical_owner {
                    BusinessOwner::Independent => None,
                    BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
                    BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
                };
                if entity.is_some_and(|entity| !is_entity_present(state, entity)) {
                    return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                        business: business.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_social_and_intelligence(
    state: &AppState,
) -> Result<(), StateValidationError> {
    for relationship in state.social.relationships() {
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
    }

    for information in state.intelligence.information() {
        match information.holder() {
            KnowledgeHolder::Character(id) => {
                if state.world.get_character(id).is_none() {
                    return Err(StateValidationError::MissingEntity {
                        context: "information holder",
                        entity: EntityRef::Character(id),
                    });
                }
            }
            KnowledgeHolder::Organization(id) => {
                if state.world.get_organization(id).is_none() {
                    return Err(StateValidationError::MissingEntity {
                        context: "information holder",
                        entity: EntityRef::Organization(id),
                    });
                }
            }
        }
        if !is_entity_present(state, information.subject()) {
            return Err(StateValidationError::MissingEntity {
                context: "information subject",
                entity: information.subject(),
            });
        }
        if let Some(source) = information.source_entity() {
            if !is_entity_present(state, source) {
                return Err(StateValidationError::MissingEntity {
                    context: "information source",
                    entity: source,
                });
            }
        }
        if information.observed_at() > information.recorded_at()
            || information.recorded_at() > state.now()
        {
            return Err(StateValidationError::InvalidInformationChronology {
                information: information.id(),
            });
        }
        if information.source_kind() == InformationSourceKind::InternalReport {
            if information.derived_from().len() != 1 || information.source_entity().is_none() {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: information.id(),
                });
            }
            let source = *information
                .derived_from()
                .iter()
                .next()
                .expect("validated internal report must have one provenance record");
            let source_record = state.intelligence.get_information(source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                },
            )?;
            if information.source_entity() != Some(source_record.holder().entity())
                || information.topic() != source_record.topic()
                || information.subject() != source_record.subject()
                || information.observed_at() != source_record.observed_at()
                || information.reliability() != source_record.reliability()
                || information.specificity() != source_record.specificity()
                || information.summary() != source_record.summary()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                });
            }
        } else if !information.derived_from().is_empty() {
            let valid_contact_kind = matches!(
                information.source_kind(),
                InformationSourceKind::PoliceContact
                    | InformationSourceKind::Lawyer
                    | InformationSourceKind::PoliticalContact
                    | InformationSourceKind::ProfessionalContact
                    | InformationSourceKind::Press
            );
            let source = information.derived_from().iter().next().copied().ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: information.id(),
                },
            )?;
            let source_record = state.intelligence.get_information(source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                },
            )?;
            if !valid_contact_kind
                || information.derived_from().len() != 1
                || state
                    .contacts
                    .disclosure_for_information(information.id())
                    .is_none()
                || information.source_entity() != Some(source_record.holder().entity())
                || !matches!(source_record.holder(), KnowledgeHolder::Character(_))
                || information.topic() != source_record.topic()
                || information.subject() != source_record.subject()
                || information.observed_at() != source_record.observed_at()
                || information.reliability() != source_record.reliability()
                || information.specificity() != source_record.specificity()
                || information.summary() != source_record.summary()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                });
            }
        }
        for source in information.derived_from() {
            let source_record = state.intelligence.get_information(*source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: *source,
                },
            )?;
            if *source >= information.id()
                || source_record.recorded_at() > information.recorded_at()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: *source,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_contacts(state: &AppState) -> Result<(), StateValidationError> {
    for contact in state.contacts.contacts() {
        let sponsor = state.world.get_organization(contact.sponsor()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let handler = state.world.get_character(contact.handler()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let source = state.world.get_character(contact.contact()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let institution = state.world.get_organization(contact.institution()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        if sponsor.kind() != OrganizationKind::Criminal
            || expected_contact_kind(institution.kind()) != Some(contact.kind())
            || contact.handler() == contact.contact()
            || contact.version() == 0
            || contact.established_at() > state.now()
            || !contact_relationship_basis_is_valid(
                contact.handler(),
                contact.contact(),
                contact.handler_to_contact(),
                contact.contact_to_handler(),
            )
        {
            return Err(StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            });
        }
        match contact.status() {
            ContactStatus::Active => {
                if contact.terminated_at().is_some()
                    || sponsor.lifecycle() != Lifecycle::Active
                    || handler.lifecycle() != Lifecycle::Active
                    || handler.organization() != Some(contact.sponsor())
                    || source.lifecycle() != Lifecycle::Active
                    || source.organization() != Some(contact.institution())
                    || institution.lifecycle() != Lifecycle::Active
                    || state
                        .contacts
                        .active_contact_for(contact.sponsor(), contact.contact())
                        .is_none_or(|current| current.id() != contact.id())
                {
                    return Err(StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    });
                }
            }
            ContactStatus::Terminated => {
                let terminated_at = contact.terminated_at().ok_or(
                    StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    },
                )?;
                if terminated_at < contact.established_at() || terminated_at > state.now() {
                    return Err(StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    });
                }
            }
        }
    }

    for disclosure in state.contacts.disclosures() {
        let contact = state.contacts.get_contact(disclosure.contact()).ok_or(
            StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        let source = state
            .intelligence
            .get_information(disclosure.source_information())
            .ok_or(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            })?;
        let disclosed = state
            .intelligence
            .get_information(disclosure.disclosed_information())
            .ok_or(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            })?;
        if disclosure.disclosed_at() < contact.established_at()
            || disclosure.disclosed_at() > state.now()
            || contact
                .terminated_at()
                .is_some_and(|terminated_at| disclosure.disclosed_at() > terminated_at)
            || source.holder() != KnowledgeHolder::Character(contact.contact())
            || source.recorded_at() > disclosure.disclosed_at()
            || source.observed_at() > disclosure.disclosed_at()
            || disclosed.holder() != KnowledgeHolder::Organization(contact.sponsor())
            || disclosed.source_kind() != resolve_information_source_kind(contact.kind())
            || disclosed.source_entity() != Some(EntityRef::Character(contact.contact()))
            || disclosed.topic() != source.topic()
            || disclosed.subject() != source.subject()
            || disclosed.observed_at() != source.observed_at()
            || disclosed.recorded_at() != disclosure.disclosed_at()
            || disclosed.reliability() != source.reliability()
            || disclosed.specificity() != source.specificity()
            || disclosed.summary() != source.summary()
            || disclosed.derived_from() != &BTreeSet::from([source.id()])
            || state
                .contacts
                .disclosure_for_information(disclosed.id())
                .is_none_or(|record| record.id() != disclosure.id())
        {
            return Err(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            });
        }
    }
    Ok(())
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
