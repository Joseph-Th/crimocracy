//! Read-only surveillance observation rendering and typed signal helpers.
//!
//! Snapshot construction and persistence stay in the parent integration owner. This child turns
//! frozen observable state into bounded player-facing prose and typed intelligence signals.

use super::*;

/// The racket kind is visibly part of the operation (a card room, a still, collectors walking
/// the street), so a watcher can tell two rackets apart even when they share one venue.
pub(super) fn enterprise_kind_label(kind: crate::enterprises::EnterpriseKind) -> &'static str {
    match kind {
        crate::enterprises::EnterpriseKind::Protection => "protection",
        crate::enterprises::EnterpriseKind::Gambling => "gambling",
        crate::enterprises::EnterpriseKind::AlcoholDistribution => "liquor",
        crate::enterprises::EnterpriseKind::Bookmaking => "bookmaking",
        crate::enterprises::EnterpriseKind::LoanSharking => "loan-sharking",
        crate::enterprises::EnterpriseKind::Fencing => "fencing",
        crate::enterprises::EnterpriseKind::Speakeasy => "speakeasy",
        crate::enterprises::EnterpriseKind::LaborRacketeering => "labor",
        crate::enterprises::EnterpriseKind::NumbersRacket => "numbers",
        crate::enterprises::EnterpriseKind::SlotMachineRoute => "slot-machine route",
        crate::enterprises::EnterpriseKind::Brothel => "brothel",
        crate::enterprises::EnterpriseKind::PrizeFighting => "prizefighting",
        crate::enterprises::EnterpriseKind::Counterfeiting => "counterfeiting",
        crate::enterprises::EnterpriseKind::Fraud => "commercial fraud",
        crate::enterprises::EnterpriseKind::AutoTheftRing => "stolen-auto ring",
        crate::enterprises::EnterpriseKind::Smuggling => "smuggling network",
    }
}

pub(super) fn patrol_summary(
    neighborhood_name: &str,
    patrol: &PatrolPatternSnapshot,
    outcome: OperationObjectiveOutcome,
    observed_at: SimTime,
    bucket_minutes: u16,
) -> String {
    if outcome == OperationObjectiveOutcome::Partial {
        let presence = patrol.current_presence.unwrap_or(patrol.baseline_presence);
        return format!(
            "Police activity around {neighborhood_name} appeared {} during the observation period; a dependable daily patrol pattern was not established.",
            police_presence_label(presence)
        );
    }
    if patrol.deployments.is_empty() {
        return format!(
            "No stable daily patrol deployment pattern was confirmed around {neighborhood_name}; visible police activity appears {} overall.",
            police_presence_label(patrol.baseline_presence)
        );
    }
    // The crew reads the precinct's daily rhythm from the watch — shift patterns, loitering
    // officers, told hours — not just the minutes it stared at one corner. The typed signal
    // carries the same deployment rhythm, so casing can protect later work planned around
    // it. A watch deliberately scheduled away from patrol still learns the rhythm it avoided.
    let observed_windows = deployment_patrol_windows(patrol);
    let windows = observed_windows
        .iter()
        .copied()
        .map(|window| approximate_patrol_window(window, bucket_minutes))
        .collect::<Vec<_>>();
    let minute = u16::try_from(observed_at.as_minutes() % u64::from(DAY_MINUTES_U16))
        .expect("minute-of-day remainder must fit u16");
    format!(
        "The crew reads a recurring patrol rhythm around {neighborhood_name} from this watch: {}. Around {}, activity was {}.",
        windows.join(", "),
        format_day_minute(rounded_day_minute(minute, bucket_minutes)),
        police_presence_label(patrol.current_presence.unwrap_or(patrol.baseline_presence))
    )
}

pub(super) fn patrol_pattern_signal(
    patrol: &PatrolPatternSnapshot,
    outcome: OperationObjectiveOutcome,
    bucket_minutes: u16,
) -> Option<InformationSignal> {
    if outcome != OperationObjectiveOutcome::Achieved || patrol.deployments.is_empty() {
        return None;
    }
    let intervals = deployment_patrol_windows(patrol)
        .into_iter()
        .flat_map(|window| approximate_patrol_intervals(window, bucket_minutes))
        .collect::<BTreeSet<_>>();
    (!intervals.is_empty()).then_some(InformationSignal::PatrolPattern { intervals })
}

fn deployment_patrol_windows(patrol: &PatrolPatternSnapshot) -> Vec<PatrolWindow> {
    patrol
        .deployments
        .iter()
        .flat_map(|deployment| deployment.windows.iter().copied())
        .collect()
}

fn approximate_patrol_intervals(
    window: PatrolWindow,
    bucket_minutes: u16,
) -> Vec<PatrolIntervalSignal> {
    let Some((start, end)) = approximate_patrol_bounds(window, bucket_minutes) else {
        return vec![
            PatrolIntervalSignal::try_new(0, DAY_MINUTES_U16)
                .expect("all-day patrol interval must be valid"),
        ];
    };
    if end > start {
        return vec![
            PatrolIntervalSignal::try_new(start, end)
                .expect("ordered patrol interval must be valid"),
        ];
    }
    let mut intervals = vec![
        PatrolIntervalSignal::try_new(start, DAY_MINUTES_U16)
            .expect("wrapped patrol tail must be valid"),
    ];
    if end > 0 {
        intervals.push(
            PatrolIntervalSignal::try_new(0, end).expect("wrapped patrol head must be valid"),
        );
    }
    intervals
}

fn approximate_patrol_window(window: PatrolWindow, bucket_minutes: u16) -> String {
    let Some((start, end)) = approximate_patrol_bounds(window, bucket_minutes) else {
        return format!("all day ({})", police_presence_label(window.presence()));
    };
    let display_end = if end == DAY_MINUTES_U16 { 0 } else { end };
    format!(
        "roughly {}-{} ({})",
        format_day_minute(start),
        format_day_minute(display_end),
        police_presence_label(window.presence())
    )
}

/// Expands an observed patrol window to containing authored observation-bucket boundaries.
/// Rounding both endpoints independently to the nearest bucket can collapse a real short window;
/// treating that collapse as all-day presence would turn a few observed minutes into twenty-four
/// hours of actionable police coverage. Containing bounds preserve uncertainty without inventing
/// coverage the observation disproves. `None` means the conservative expansion covers the full day.
fn approximate_patrol_bounds(window: PatrolWindow, bucket_minutes: u16) -> Option<(u16, u16)> {
    let bucket_minutes = u32::from(bucket_minutes);
    let day = u32::from(DAY_MINUTES_U16);
    let start = u32::from(window.start().value());
    let end = start + u32::from(window.duration_minutes());
    let approximate_start = start / bucket_minutes * bucket_minutes;
    let approximate_end = end.div_ceil(bucket_minutes) * bucket_minutes;
    if approximate_end - approximate_start >= day {
        return None;
    }
    let start = u16::try_from(approximate_start % day)
        .expect("bucketed patrol start must fit minute-of-day width");
    let end = if approximate_end <= day {
        u16::try_from(approximate_end).expect("same-day patrol end must fit interval width")
    } else {
        u16::try_from(approximate_end % day)
            .expect("wrapped patrol end must fit minute-of-day width")
    };
    debug_assert_ne!(start, end);
    Some((start, end))
}

fn rounded_day_minute(minute: u16, bucket_minutes: u16) -> u16 {
    let bucket = u32::from(bucket_minutes);
    let rounded = (u32::from(minute) + bucket / 2) / bucket * bucket;
    u16::try_from(rounded % u32::from(DAY_MINUTES_U16)).expect("rounded day minute must fit u16")
}

fn format_day_minute(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn police_presence_label(rating: Rating) -> &'static str {
    rating.police_presence_label()
}

pub(super) fn business_access_summary(
    name: &str,
    functions: &BTreeSet<BusinessFunction>,
) -> String {
    let access = functions
        .iter()
        .map(|function| function.description())
        .collect::<Vec<_>>();
    if access.is_empty() {
        format!("Surveillance of {name} identified no specialized operating access.")
    } else {
        format!(
            "Surveillance of {name} confirmed operating access associated with {}.",
            access.join(", ")
        )
    }
}

pub(super) fn character_summary(
    name: &str,
    organization: Option<&(OrganizationId, String)>,
    supervisor: Option<&(CharacterId, String)>,
) -> String {
    let affiliation = organization
        .map(|(_, organization)| format!("regularly associated with {organization}"))
        .unwrap_or_else(|| "not regularly associated with a known organization".to_owned());
    let reporting = supervisor
        .map(|(_, supervisor)| format!(" An apparent reporting contact is {supervisor}."))
        .unwrap_or_default();
    format!("Surveillance observed {name} {affiliation}.{reporting}")
}

pub(super) fn is_law_enforcement_authority(kind: OrganizationKind) -> bool {
    matches!(
        kind,
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    )
}

pub(super) fn resolve_known_authority_case_activity(
    state: &AppState,
    authority: OrganizationId,
    surveiller: OrganizationId,
) -> Option<CaseActivitySignal> {
    state
        .legal
        .investigations_for_owner(authority)
        .filter(|case| {
            case.origin().is_some_and(|origin| {
                crate::legal::investigation_system::case_origin_responsible_organization(
                    state, origin,
                ) == Some(surveiller)
            })
        })
        .map(|case| crate::legal::case_knowledge::activity_for_status(case.status()))
        .fold(None, |aggregate, activity| {
            Some(match (aggregate, activity) {
                (Some(CaseActivitySignal::Active), _) | (_, CaseActivitySignal::Active) => {
                    CaseActivitySignal::Active
                }
                (Some(CaseActivitySignal::Shelved), _) | (_, CaseActivitySignal::Shelved) => {
                    CaseActivitySignal::Shelved
                }
                (None | Some(CaseActivitySignal::Closed), CaseActivitySignal::Closed) => {
                    CaseActivitySignal::Closed
                }
            })
        })
}

pub(super) fn authority_sightline_summary(
    name: &str,
    activity: CaseActivitySignal,
    outcome: OperationObjectiveOutcome,
) -> String {
    // The observation reports only visible authority activity tied to a case caused by the
    // surveilling organization's own activity; it never reveals evidence, subjects, or internals.
    if outcome == OperationObjectiveOutcome::Partial {
        return format!(
            "Visible activity around {name} remained difficult to judge; a dependable read on whether the case is still being actively developed was not established."
        );
    }
    // Dependable reads share the same display prefix as investigator-held case knowledge so
    // the two player-facing channels describe case activity consistently without parsing prose.
    let prose = match activity {
        CaseActivitySignal::Active => format!(
            "Detectives around {name} appear to be actively developing the case connected to your recent activity. The matter has not gone quiet."
        ),
        CaseActivitySignal::Shelved => format!(
            "No active case machinery connected to your recent activity was observed around {name}; the matter appears to have been shelved and routine police functions continue."
        ),
        CaseActivitySignal::Closed => format!(
            "No active case machinery connected to your recent activity was observed around {name}; the matter appears closed."
        ),
    };
    format!(
        "{} {prose}",
        crate::legal::case_knowledge::case_activity_summary_prefix(activity)
    )
}

pub(super) fn authority_sightline_signal(
    activity: CaseActivitySignal,
    outcome: OperationObjectiveOutcome,
) -> Option<CaseActivitySignal> {
    (outcome == OperationObjectiveOutcome::Achieved).then_some(activity)
}

pub(super) fn organization_summary(
    name: &str,
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> String {
    let observed = observed_organization_members(active_members, outcome)
        .iter()
        .map(|(_, member)| member.as_str())
        .collect::<Vec<_>>();
    if observed.is_empty() {
        format!("Surveillance of {name} did not identify a recurring active affiliate.")
    } else {
        format!(
            "Recurring activity around {name} included {}.",
            observed.join(", ")
        )
    }
}

pub(super) fn organization_personnel_signal(
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> Option<InformationSignal> {
    let characters = observed_organization_members(active_members, outcome)
        .iter()
        .map(|(character, _)| *character)
        .collect::<BTreeSet<_>>();
    (!characters.is_empty()).then_some(InformationSignal::PersonnelPresence { characters })
}

fn observed_organization_members(
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> &[(CharacterId, String)] {
    let limit = if outcome == OperationObjectiveOutcome::Achieved {
        3
    } else {
        1
    };
    &active_members[..active_members.len().min(limit)]
}

pub(super) fn investigation_summary(
    title: &str,
    owner_name: &str,
    status: InvestigationStatus,
    lead: Option<&(CharacterId, String)>,
    outcome: OperationObjectiveOutcome,
) -> String {
    // A Partial read is undependable: like the authority-sightline channel, it hedges
    // instead of stating the live file status, so watching the file directly cannot yield
    // precisely the knowledge the indirect channel deliberately withholds.
    if outcome == OperationObjectiveOutcome::Partial {
        return format!(
            "Visible activity around the {title} file remained difficult to judge; a dependable read on whether the matter is still being actively developed was not established."
        );
    }
    let lead_clause = if outcome == OperationObjectiveOutcome::Achieved {
        lead.map(|(_, name)| format!(" {name} appears to be directing the visible work."))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "Visible activity around the {title} file indicates the matter is {} under {owner_name}.{lead_clause}",
        investigation_status_label(status)
    )
}

pub(super) fn investigation_case_signal(
    status: InvestigationStatus,
    outcome: OperationObjectiveOutcome,
) -> Option<InformationSignal> {
    (outcome == OperationObjectiveOutcome::Achieved).then(|| {
        InformationSignal::CaseActivity(crate::legal::case_knowledge::activity_for_status(status))
    })
}

pub(super) fn enterprise_summary(
    kind: crate::enterprises::EnterpriseKind,
    organization_name: &str,
    manager_name: &str,
    location_name: &str,
    status: EnterpriseStatus,
) -> String {
    format!(
        "Observed {} activity at {location_name} appears {} under {manager_name} for {organization_name}.",
        enterprise_kind_label(kind),
        enterprise_status_label(status)
    )
}

pub(super) fn enterprise_location_name(state: &AppState, location: EnterpriseLocation) -> String {
    match location {
        EnterpriseLocation::Neighborhood(neighborhood) => state
            .world
            .get_neighborhood(neighborhood)
            .expect("enterprise surveillance target must reference a persisted neighborhood")
            .name()
            .to_owned(),
        EnterpriseLocation::Business(business) => state
            .world
            .get_business(business)
            .expect("enterprise surveillance target must reference a persisted business")
            .name()
            .to_owned(),
    }
}

fn investigation_status_label(status: InvestigationStatus) -> &'static str {
    match status {
        InvestigationStatus::Active => "active",
        InvestigationStatus::Suspended => "quiet or suspended",
        InvestigationStatus::Closed => "closed",
    }
}

fn enterprise_status_label(status: EnterpriseStatus) -> &'static str {
    match status {
        EnterpriseStatus::Active => "active",
        EnterpriseStatus::Suspended => "inactive or suspended",
        EnterpriseStatus::Retired => "closed and retired",
    }
}

pub(super) fn operation_status_label(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Authorized => "planned but not yet underway",
        OperationStatus::InProgress => "currently underway",
        OperationStatus::AwaitingDecision => "paused pending direction",
        OperationStatus::Completed => "completed",
        OperationStatus::Aborted => "aborted",
    }
}
