//! Internal records available only after a successful infiltration, never from public watch.
use super::*;
use crate::finance::helpers::format_money_cents;
use crate::world::BusinessOwner;

fn finding(
    topic: InformationTopic,
    subject: EntityRef,
    summary: String,
    label: String,
) -> SurveillanceObservation {
    SurveillanceObservation {
        topic,
        subject,
        reliability: Reliability::GenerallyReliable,
        specificity: Specificity::Specific,
        signal: None,
        summary,
        finding: label,
    }
}

/// The target bounds the records a planted source can see. An inside job does not enumerate
/// unrelated organizations, bank accounts, police evidence or hidden city-wide holdings.
pub(super) fn internal_findings(
    state: &AppState,
    target: EntityRef,
) -> Vec<SurveillanceObservation> {
    match target {
        EntityRef::Business(id) => {
            let Some(business) = state.world.get_business(id) else {
                return Vec::new();
            };
            let name = business.name();
            let owner = match business.owner() {
                BusinessOwner::Independent => "an independent proprietor".to_owned(),
                BusinessOwner::Character(owner) => state
                    .world
                    .get_character(owner)
                    .expect("business owner must exist")
                    .name()
                    .to_owned(),
                BusinessOwner::Organization(owner) => state
                    .world
                    .get_organization(owner)
                    .expect("business owner must exist")
                    .name()
                    .to_owned(),
            };
            let financials = state
                .economy
                .latest_cycle_for_business_version(id, business.version())
                .map_or_else(
                    || {
                        "No completed operating cycle in the current ownership period was found."
                            .to_owned()
                    },
                    |cycle| {
                        format!(
                            "Last settled cycle at minute {}: gross {}, costs {}, net {}.",
                            cycle.occurred_at().as_minutes(),
                            format_money_cents(cycle.gross_revenue().cents()),
                            format_money_cents(cycle.operating_cost().cents()),
                            format_money_cents(cycle.net_cash().cents())
                        )
                    },
                );
            let mut observations = vec![finding(
                InformationTopic::FinancialPerformance,
                target,
                format!("Inside records at {name} identify the owner as {owner}. {financials}"),
                format!("internal books of {name}"),
            )];
            match business.owner() {
                BusinessOwner::Organization(id) => observations.push(finding(InformationTopic::MarketAccess, EntityRef::Organization(id),
                    format!("Internal ownership records at {name} identify {owner} as the controlling organization."), format!("commercial records of {owner}"))),
                BusinessOwner::Character(id) => observations.push(finding(InformationTopic::Personnel, EntityRef::Character(id),
                    format!("Internal ownership records at {name} identify {owner} as its proprietor."), format!("personnel file on {owner}"))),
                BusinessOwner::Independent => {},
            }
            observations
        }
        EntityRef::Organization(id) => {
            let Some(organization) = state.world.get_organization(id) else {
                return Vec::new();
            };
            // Legal authorities retain the existing own-case visibility boundary. Private
            // organizational records of criminal/commercial outfits reveal their leadership.
            if !matches!(
                organization.kind(),
                OrganizationKind::Criminal | OrganizationKind::Commercial
            ) {
                return Vec::new();
            }
            let mut managers = state
                .world
                .characters_in_organization(id)
                .filter(|member| {
                    member.supervisor().is_none()
                        || state.delegation.active_for_manager(member.id()).is_some()
                })
                .collect::<Vec<_>>();
            managers.sort_by_key(|member| (member.supervisor().is_some(), member.id()));
            managers
                .into_iter()
                .take(3)
                .map(|member| {
                    let reports_to = member.supervisor().map(|supervisor| {
                        state
                            .world
                            .get_character(supervisor)
                            .expect("supervisor must exist")
                            .name()
                    });
                    let authority = state.delegation.active_for_manager(member.id());
                    let remit = authority
                        .map_or("no standing delegated mandate".to_owned(), |mandate| {
                            format!("delegated scopes {:?}", mandate.scopes())
                        });
                    finding(
                        InformationTopic::Personnel,
                        EntityRef::Character(member.id()),
                        format!(
                            "Inside personnel records of {}: {} reports to {}; {}.",
                            organization.name(),
                            member.name(),
                            reports_to.unwrap_or("no supervisor"),
                            remit
                        ),
                        format!("personnel file on {}", member.name()),
                    )
                })
                .collect()
        }
        EntityRef::Enterprise(id) => {
            let Some(enterprise) = state.enterprises.get_enterprise(id) else {
                return Vec::new();
            };
            let name = enterprise_kind_label(enterprise.kind());
            let location = enterprise_location_name(state, enterprise.location());
            let supports = enterprise
                .supporting_businesses()
                .iter()
                .map(|id| {
                    state
                        .world
                        .get_business(*id)
                        .expect("support business must exist")
                        .name()
                })
                .collect::<Vec<_>>();
            let network = if supports.is_empty() {
                "no separate supporting venues".to_owned()
            } else {
                supports.join(", ")
            };
            let mut observations = vec![finding(
                InformationTopic::MarketAccess,
                target,
                format!(
                    "Inside records of {name} at {location} identify its supporting network: {network}."
                ),
                format!("support network of {name} at {location}"),
            )];
            if let Some(cycle) = state.enterprises.latest_cycle(id) {
                observations.push(finding(InformationTopic::FinancialPerformance, target,
                    format!("Inside books of {name} at {location}: last settlement at minute {}, gross {}, cost {}, net {}.",
                        cycle.occurred_at().as_minutes(),
                        format_money_cents(cycle.gross_revenue().cents()),
                        format_money_cents(cycle.operating_cost().cents()),
                        format_money_cents(cycle.net_cash().cents())),
                    format!("internal books of {name} at {location}")));
            }
            observations
        }
        EntityRef::Character(_)
        | EntityRef::Neighborhood(_)
        | EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => Vec::new(),
    }
}
