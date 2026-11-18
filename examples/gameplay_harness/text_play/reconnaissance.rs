//! The selection boundary for broad reconnaissance targets. The production
//! operation validator owns viability and knowledge checks, but the text client
//! must never resolve hidden record identities simply because an ID was typed.

use super::*;
use crimocracy::core::id::NeighborhoodId;

impl Session<'_> {
    pub(super) fn known_surveillance_targets(&self) -> String {
        let mut out = String::new();
        out.push_str("Public businesses/districts: see 'directory' and 'districts'.\n");
        out.push_str("Identified people: see 'known-people'.\n");
        for id in self.known_organization_ids() {
            if let Some(record) = self.state.world().get_organization(id) {
                writeln!(
                    out,
                    "org-{} | {} | identified institution",
                    id.raw(),
                    record.name()
                )
                .unwrap();
            }
        }
        for id in self.known_operation_ids() {
            if let Some(record) = self.state.operations().get_operation(id) {
                if record.responsible_organization() == self.player {
                    writeln!(
                        out,
                        "operation-{} | your operation: {}",
                        id.raw(),
                        record.title()
                    )
                    .unwrap();
                } else {
                    writeln!(out, "operation-{} | learned operation", id.raw()).unwrap();
                }
            }
        }
        let mut enterprise_observations = BTreeMap::new();
        for info in self.organization_information() {
            if let EntityRef::Enterprise(id) = info.subject()
                && matches!(
                    info.signal(),
                    Some(InformationSignal::EnterpriseLocation(_))
                )
            {
                enterprise_observations
                    .entry(id)
                    .and_modify(
                        |current: &mut &crimocracy::intelligence::InformationRecord| {
                            if (info.observed_at(), info.id())
                                > (current.observed_at(), current.id())
                            {
                                *current = info;
                            }
                        },
                    )
                    .or_insert(info);
            }
        }
        for id in self.known_enterprise_ids() {
            if let Some(record) = self.state.enterprises().get_enterprise(id) {
                if record.organization() == self.player {
                    writeln!(
                        out,
                        "enterprise-{} | your {:?} at {}",
                        id.raw(),
                        record.kind(),
                        self.enterprise_location_label(record.location())
                    )
                    .unwrap();
                } else {
                    if let Some(info) = enterprise_observations.get(&id) {
                        writeln!(out, "enterprise-{} | learned enterprise | observed {} | {} (information-{})", id.raw(), time_label(info.observed_at().as_minutes()), info.summary(), info.id().raw()).unwrap();
                    } else {
                        writeln!(
                            out,
                            "enterprise-{} | learned enterprise; location unverified",
                            id.raw()
                        )
                        .unwrap();
                    }
                }
            }
        }
        for id in self.known_investigation_ids() {
            // A named case may be learned while its actual status and title
            // remain hidden. Only its known ID is presented until observed.
            writeln!(
                out,
                "investigation-{} | learned case; status unverified",
                id.raw()
            )
            .unwrap();
        }
        out
    }

    /// All reconnaissance routes resolve known IDs before accessing the
    /// associated world/administrative records, except deliberately public
    /// business and neighborhood directories.
    pub(super) fn reconnaissance_target(
        &self,
        raw: &str,
    ) -> Result<(EntityRef, String, Option<NeighborhoodId>), String> {
        if raw.starts_with("district-") {
            let number = parse_number(raw, "district")?;
            let record = self
                .state
                .world()
                .neighborhoods()
                .find(|record| record.id().raw() == number)
                .ok_or("district is not in the public directory")?;
            return Ok((
                EntityRef::Neighborhood(record.id()),
                record.name().to_owned(),
                Some(record.id()),
            ));
        }
        if raw.starts_with("char-") {
            let number = parse_number(raw, "char")?;
            let id = self
                .known_character(number)
                .ok_or("character is not identified by your organization")?;
            let record = self
                .state
                .world()
                .get_character(id)
                .ok_or("identified character disappeared")?;
            return Ok((EntityRef::Character(id), record.name().to_owned(), None));
        }
        if raw.starts_with("org-") {
            let number = parse_number(raw, "org")?;
            let id = self
                .known_organization_ids()
                .into_iter()
                .find(|id| id.raw() == number)
                .ok_or("institution is not identified by your organization")?;
            let record = self
                .state
                .world()
                .get_organization(id)
                .ok_or("identified institution disappeared")?;
            return Ok((EntityRef::Organization(id), record.name().to_owned(), None));
        }
        if raw.starts_with("operation-") {
            let number = parse_number(raw, "operation")?;
            let id = self
                .known_operation_ids()
                .into_iter()
                .find(|id| id.raw() == number)
                .ok_or("operation is not known to your organization")?;
            return Ok((
                EntityRef::Operation(id),
                format!("known operation-{number}"),
                None,
            ));
        }
        if raw.starts_with("investigation-") {
            let number = parse_number(raw, "investigation")?;
            let id = self
                .known_investigation_ids()
                .into_iter()
                .find(|id| id.raw() == number)
                .ok_or("investigation is not known to your organization")?;
            return Ok((
                EntityRef::Investigation(id),
                format!("known investigation-{number}"),
                None,
            ));
        }
        if raw.starts_with("enterprise-") {
            let number = parse_number(raw, "enterprise")?;
            let id = self
                .known_enterprise_ids()
                .into_iter()
                .find(|id| id.raw() == number)
                .ok_or("enterprise is not known to your organization")?;
            return Ok((
                EntityRef::Enterprise(id),
                format!("known enterprise-{number}"),
                None,
            ));
        }
        if raw.contains('-') && !raw.starts_with("business-") {
            return Err("unsupported surveillance target; see 'known-targets'".to_owned());
        }
        let number = parse_number(raw, "business")?;
        let id = self
            .known_business(number)
            .ok_or("business is not in the public city directory")?;
        let record = self
            .state
            .world()
            .get_business(id)
            .ok_or("public business disappeared")?;
        Ok((
            EntityRef::Business(id),
            record.name().to_owned(),
            Some(record.neighborhood()),
        ))
    }
}
