//! Prosecution-case and referral mutation/index maintenance.

use super::super::*;

impl LegalState {
    pub(in crate::legal) fn insert_prosecution_case(
        &mut self,
        case: ProsecutionCaseRecord,
        referral: ProsecutionReferralRecord,
    ) {
        let case_id = case.id();
        let referral_id = referral.id();
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert_eq!(referral.prosecution_case(), case_id);
        debug_assert_eq!(case.initial_referral(), referral_id);
        debug_assert_eq!(case.referrals(), &BTreeSet::from([referral_id]));
        debug_assert_eq!(case.evidence(), referral.evidence());
        if let Some(prosecutor) = case.assigned_prosecutor() {
            self.indexes
                .prosecutions
                .reviewing_cases_by_prosecutor
                .entry(prosecutor)
                .or_default()
                .insert(case_id);
        } else {
            self.indexes
                .prosecutions
                .reviewing_without_prosecutor
                .insert(case_id);
        }
        let previous_open = self
            .indexes
            .prosecutions
            .open_by_arrest_office
            .insert((case.arrest(), case.prosecutor_office()), case_id);
        debug_assert!(previous_open.is_none());
        self.indexes
            .prosecutions
            .referrals_by_case
            .entry(case_id)
            .or_default()
            .insert(referral_id);
        let previous_case = self.prosecution_cases.insert(case_id, case);
        let previous_referral = self.prosecution_referrals.insert(referral_id, referral);
        debug_assert!(previous_case.is_none());
        debug_assert!(previous_referral.is_none());
    }
    pub(in crate::legal) fn add_prosecution_referral(
        &mut self,
        referral: ProsecutionReferralRecord,
    ) {
        let referral_id = referral.id();
        let case_id = referral.prosecution_case();
        let case = self
            .prosecution_cases
            .get_mut(&case_id)
            .expect("validated prosecution case disappeared before referral commit");
        for evidence in referral.evidence() {
            let inserted = case.referrals.evidence.insert(*evidence);
            debug_assert!(inserted, "supplemental referral must add new evidence");
        }
        case.referrals.referrals.insert(referral_id);
        case.version = advance_version_preflighted(case.version);
        self.indexes
            .prosecutions
            .referrals_by_case
            .entry(case_id)
            .or_default()
            .insert(referral_id);
        let previous = self.prosecution_referrals.insert(referral_id, referral);
        debug_assert!(previous.is_none());
    }
    pub(in crate::legal) fn apply_prosecution_resolution(
        &mut self,
        id: ProsecutionCaseId,
        resolution: ProsecutionCaseResolution,
        resolved_at: SimTime,
        prosecutor: CharacterId,
        information: InformationId,
        report: ReportId,
    ) {
        let (arrest, office, assigned) = {
            let case = self
                .prosecution_cases
                .get(&id)
                .expect("validated prosecution case disappeared before resolution commit");
            debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
            (
                case.arrest(),
                case.prosecutor_office(),
                case.assigned_prosecutor(),
            )
        };
        let removed = self
            .indexes
            .prosecutions
            .open_by_arrest_office
            .remove(&(arrest, office));
        debug_assert_eq!(removed, Some(id));
        match assigned {
            Some(assigned) => {
                debug_assert_eq!(assigned, prosecutor);
                if let Some(cases) = self
                    .indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .get_mut(&assigned)
                {
                    let removed = cases.remove(&id);
                    debug_assert!(removed);
                    if cases.is_empty() {
                        self.indexes
                            .prosecutions
                            .reviewing_cases_by_prosecutor
                            .remove(&assigned);
                    }
                }
            }
            None => {
                debug_assert!(
                    false,
                    "validated prosecution resolution must have an assignee"
                );
                self.indexes
                    .prosecutions
                    .reviewing_without_prosecutor
                    .remove(&id);
            }
        }
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before resolution commit");
        case.context.assigned_prosecutor = None;
        case.lifecycle.status = resolution.status();
        case.lifecycle.resolved_at = Some(resolved_at);
        case.resolution_artifacts.resolution_information = Some(information);
        case.resolution_artifacts.resolution_report = Some(report);
        case.resolution_artifacts.resolution_prosecutor = Some(prosecutor);
        case.version = advance_version_preflighted(case.version);
    }

    pub(in crate::legal) fn set_prosecution_case_prosecutor(
        &mut self,
        id: ProsecutionCaseId,
        prosecutor: CharacterId,
    ) {
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before staffing commit");
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert!(case.assigned_prosecutor().is_none());
        case.context.assigned_prosecutor = Some(prosecutor);
        case.version = advance_version_preflighted(case.version);
        let removed = self
            .indexes
            .prosecutions
            .reviewing_without_prosecutor
            .remove(&id);
        debug_assert!(removed);
        self.indexes
            .prosecutions
            .reviewing_cases_by_prosecutor
            .entry(prosecutor)
            .or_default()
            .insert(id);
    }

    pub(in crate::legal) fn release_prosecution_case_prosecutor_for_detention(
        &mut self,
        id: ProsecutionCaseId,
        prosecutor: CharacterId,
    ) {
        self.release_prosecution_case_prosecutor_runtime(id, prosecutor);
    }

    pub(super) fn release_prosecution_case_prosecutor_runtime(
        &mut self,
        id: ProsecutionCaseId,
        prosecutor: CharacterId,
    ) {
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before staffing release");
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert_eq!(case.assigned_prosecutor(), Some(prosecutor));
        case.context.assigned_prosecutor = None;
        case.version = advance_version_preflighted(case.version);
        if let Some(cases) = self
            .indexes
            .prosecutions
            .reviewing_cases_by_prosecutor
            .get_mut(&prosecutor)
        {
            let removed = cases.remove(&id);
            debug_assert!(removed);
            if cases.is_empty() {
                self.indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .remove(&prosecutor);
            }
        }
        self.indexes
            .prosecutions
            .reviewing_without_prosecutor
            .insert(id);
    }
}
