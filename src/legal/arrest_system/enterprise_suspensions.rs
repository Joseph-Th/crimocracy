//! Arrest disrupts the managerial assignments actually held by the detainee.
//! A release ends custody, not the owner's decision to suspend an enterprise.

use super::ArrestError;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, EnterpriseId, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::enterprises::enterprise_execution::{
    EnterpriseError, ValidatedEnterpriseStatusChange, validate_suspend_enterprise,
};
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct CustodyEnterpriseSuspensions {
    manager: CharacterId,
    suspensions: Vec<(EnterpriseId, ValidatedEnterpriseStatusChange)>,
    notices: Vec<ValidatedReport>,
}

impl std::fmt::Debug for CustodyEnterpriseSuspensions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CustodyEnterpriseSuspensions")
            .field("manager", &self.manager)
            .field(
                "enterprises",
                &self
                    .suspensions
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn managed_enterprises(state: &AppState, manager: CharacterId) -> BTreeSet<EnterpriseId> {
    // An active enterprise must belong to its manager's unique active mandate.
    // Traverse that mandate's indexed enterprises rather than every city racket.
    state
        .delegation()
        .active_for_manager(manager)
        .into_iter()
        .flat_map(|mandate| state.enterprises().active_for_mandate(mandate.id()))
        .map(|enterprise| enterprise.id())
        .collect()
}

fn suspension_error(manager: CharacterId, error: EnterpriseError) -> ArrestError {
    if let EnterpriseError::VersionCapacity(error) = error {
        ArrestError::VersionCapacity(error)
    } else {
        ArrestError::CustodyResponsibilitiesChanged { character: manager }
    }
}

impl CustodyEnterpriseSuspensions {
    pub(super) fn prepare(state: &AppState, manager: CharacterId) -> Result<Self, ArrestError> {
        let mut suspensions = Vec::new();
        let mut by_owner = BTreeMap::<OrganizationId, BTreeSet<EntityRef>>::new();
        for id in managed_enterprises(state, manager) {
            let record = state
                .enterprises
                .get_enterprise(id)
                .expect("active enterprise must exist");
            suspensions.push((
                id,
                validate_suspend_enterprise(state, id)
                    .map_err(|error| suspension_error(manager, error))?,
            ));
            by_owner
                .entry(record.organization())
                .or_default()
                .insert(EntityRef::Enterprise(id));
        }
        let name = state
            .world
            .get_character(manager)
            .ok_or(ArrestError::MissingCharacter(manager))?
            .name();
        let notices = by_owner.into_iter().map(|(recipient, mut entities)| {
            let count = entities.len();
            entities.insert(EntityRef::Character(manager));
            validate_record_report(state, ReportDraft {
                recipient,
                kind: ReportKind::Legal,
                title: "Manager detained: enterprises suspended".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Exception,
                    summary: format!("{name} was detained; {count} managed enterprises suspended. Release does not reopen them automatically."),
                    sources: Vec::new(),
                    entities,
                    decision: None,
                }],
            })
        }).collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            manager,
            suspensions,
            notices,
        })
    }

    pub(super) fn ensure_current(&self, state: &AppState) -> Result<(), ArrestError> {
        if managed_enterprises(state, self.manager)
            != self.suspensions.iter().map(|(id, _)| *id).collect()
        {
            return Err(ArrestError::CustodyResponsibilitiesChanged {
                character: self.manager,
            });
        }
        for (_, suspension) in &self.suspensions {
            suspension
                .ensure_suspension_current(state)
                .map_err(|error| suspension_error(self.manager, error))?;
        }
        Ok(())
    }

    pub(super) fn id_budget(&self) -> (IdKind, u32) {
        (
            IdKind::Report,
            u32::try_from(self.notices.len()).expect("organization count fits ID capacity"),
        )
    }

    pub(super) fn commit(self, state: &mut AppState) {
        for (_, suspension) in self.suspensions {
            suspension.commit_suspension_preflighted(state);
        }
        for notice in self.notices {
            notice
                .commit(state)
                .expect("custody suspension notice was preflighted");
        }
    }
}
