//! Player time advancement, with interruption and terminal-clock semantics kept in one place.

use super::*;

/// Clamp requested one-minute ticks to the finite simulation clock before mutation. Without
/// this preflight a multi-minute command could advance partway, then return a rejection when
/// the next minute was no longer representable.
fn bounded_advance_span(
    now: crimocracy::core::time::SimTime,
    requested: u64,
) -> Result<u64, String> {
    let available = u64::MAX - now.as_minutes();
    if available == 0 {
        return Err("simulation clock is exhausted; no further advance is possible".to_owned());
    }
    Ok(requested.min(available))
}

impl Session<'_> {
    pub(super) fn execute_advance(
        &mut self,
        duration: &str,
        uninterrupted: bool,
    ) -> Result<String, String> {
        let requested = parse_advance(duration)?;
        if self.state.player_organization_defeated() {
            return Err("Campaign over: your organization dissolved".to_owned());
        }
        if let Some(event) = self.state.campaign_victory() {
            return Err(format!("Campaign over: {}", event.summary()));
        }
        if self
            .state
            .decisions()
            .pending_for_recipient(self.player)
            .next()
            .is_some()
        {
            return Err("resolve your pending decisions before advancing".to_owned());
        }
        let span = bounded_advance_span(self.state.now(), requested)?;
        let mut advanced = 0;
        let mut new_reports = 0;
        let mut reason = None;
        let mut deferred_custody = 0_u32;
        let mut deferred_releases = 0_u32;
        let mut deferred_shutdowns = 0_u32;
        let mut deferred_losses = 0_u32;
        let mut deferred_payroll = 0_u32;
        let mut deferred_operations = 0_u32;
        let mut deferred_urgent = 0_u32;
        for _ in 0..span {
            let previous = self
                .state
                .reports()
                .reports_for(self.player)
                .next_back()
                .map(|r| r.id());
            let tick = run_tick(self.registry, &mut self.state).map_err(|e| e.to_string())?;
            advanced += 1;
            let reports: Vec<_> = self
                .state
                .reports()
                .reports_for_after(self.player, previous)
                .collect();
            new_reports += reports.len();
            let urgent = reports.iter().any(|report| {
                report.kind() != crimocracy::reports::ReportKind::ExecutiveBrief
                    && report.entries().iter().any(|entry| {
                        entry.attention.can_auto_pause()
                            && self
                                .state
                                .attention_settings()
                                .is_auto_pause_enabled(entry.attention)
                    })
            });
            let operation_ended = tick
                .resolved_operations
                .iter()
                .chain(&tick.aborted_operations)
                .any(|id| {
                    self.state
                        .operations()
                        .get_operation(*id)
                        .is_some_and(|op| op.responsible_organization() == self.player)
                });
            let own_custody_event = |arrests: &[ArrestId]| {
                arrests.iter().any(|id| {
                    self.state.legal().get_arrest(*id).is_some_and(|arrest| {
                        self.state
                            .world()
                            .get_character(arrest.character())
                            .is_some_and(|person| person.organization() == Some(self.player))
                    })
                })
            };
            let own_arrest = own_custody_event(&tick.evidence_arrests);
            let own_release = own_custody_event(&tick.custody_releases);
            let own_payroll_shortfall = tick.payrolls.iter().any(|payroll| {
                payroll.organization() == self.player && payroll.short() > Money::ZERO
            });
            let own_business_suspended = tick.business_cycles.iter().any(|id| {
                let cycle = self.state.economy().get_cycle(*id).expect("settled cycle");
                cycle.owner() == crimocracy::world::BusinessOwner::Organization(self.player)
                    && self
                        .state
                        .economy()
                        .get_business_economy(cycle.business())
                        .is_some_and(|business| {
                            business.status()
                                == crimocracy::economy::BusinessOperatingStatus::Suspended
                        })
            });
            let own_racket_suspended = tick.enterprise_cycles.iter().any(|id| {
                let cycle = self
                    .state
                    .enterprises()
                    .get_cycle(*id)
                    .expect("settled cycle");
                self.state
                    .enterprises()
                    .get_enterprise(cycle.enterprise())
                    .is_some_and(|racket| {
                        racket.organization() == self.player
                            && racket.status() == EnterpriseStatus::Suspended
                    })
            });
            let own_managerial_shutdown =
                tick.managerial_overstretch_suspensions.iter().any(|id| {
                    self.state
                        .enterprises()
                        .get_enterprise(*id)
                        .is_some_and(|racket| racket.organization() == self.player)
                });
            // A first loss is an actionable change, not a routine daily settlement. Do not
            // repeatedly interrupt an acknowledged losing streak or reveal rival results.
            let own_new_racket_loss = tick.enterprise_cycles.iter().any(|id| {
                let enterprises = self.state.enterprises();
                let cycle = enterprises.get_cycle(*id).expect("settled cycle");
                enterprises
                    .get_enterprise(cycle.enterprise())
                    .is_some_and(|racket| racket.organization() == self.player)
                    && enterprises.cycle_starts_current_loss_streak(*id)
            });
            let pause_exceptions = self
                .state
                .attention_settings()
                .is_auto_pause_enabled(crimocracy::core::attention::AttentionClass::Exception);
            if uninterrupted {
                deferred_custody += u32::from(own_arrest);
                deferred_releases += u32::from(own_release);
                deferred_shutdowns += u32::from(
                    own_business_suspended || own_managerial_shutdown || own_racket_suspended,
                );
                deferred_losses += u32::from(own_new_racket_loss);
                deferred_payroll += u32::from(own_payroll_shortfall);
                deferred_operations += u32::from(operation_ended);
                deferred_urgent += u32::from(urgent);
            }
            if self.state.player_organization_defeated() {
                reason = Some("Campaign over: your organization dissolved");
            } else if self.state.campaign_victory().is_some() {
                reason = Some("Campaign over: victory");
            } else if self
                .state
                .decisions()
                .pending_for_recipient(self.player)
                .next()
                .is_some()
            {
                reason = Some("leadership decision pending");
            } else if !uninterrupted && own_arrest {
                reason = Some("your member entered custody; review 'counsel' and 'contacts'");
            } else if !uninterrupted && own_release {
                reason = Some("your member was released from custody; review 'counsel'");
            } else if !uninterrupted && pause_exceptions && own_payroll_shortfall {
                reason = Some("payroll ran short; review 'treasury', 'businesses' and 'accounts'");
            } else if !uninterrupted && pause_exceptions && own_business_suspended {
                reason = Some(
                    "your storefront has suspended operations; review 'businesses' and 'enterprises'",
                );
            } else if !uninterrupted && pause_exceptions && own_managerial_shutdown {
                reason = Some(
                    "a captain cannot supervise the current racket workload; review 'people', 'delegate', 'enterprises' and 'prospects'",
                );
            } else if !uninterrupted && pause_exceptions && own_racket_suspended {
                reason = Some("your racket has suspended operations; review 'enterprises'");
            } else if !uninterrupted && pause_exceptions && own_new_racket_loss {
                reason = Some(
                    "your racket has entered a losing cycle; review 'enterprises' and 'racket-outlook'",
                );
            } else if !uninterrupted && urgent {
                reason = Some("high-priority report");
            } else if !uninterrupted && operation_ended {
                reason = Some("your operation reached a terminal outcome");
            }
            if reason.is_some() {
                break;
            }
        }
        let mut out = String::new();
        writeln!(
            out,
            "Advanced {advanced} minute(s) to {}. {new_reports} new report(s).",
            time_label(self.state.now().as_minutes())
        )
        .unwrap();
        if let Some(reason) = reason {
            writeln!(out, "Paused: {reason}.").unwrap();
        }
        if uninterrupted {
            writeln!(
                out,
                "Uninterrupted review: {deferred_custody} custody event(s), {deferred_releases} release(s), {deferred_shutdowns} shutdown event(s), {deferred_losses} new loss streak(s), {deferred_payroll} payroll shortfall(s), {deferred_operations} completed/aborted operation minute(s), {deferred_urgent} priority-report minute(s). All events were simulated. Review 'reports', 'counsel', 'recovery' and 'accounts'."
            )
            .unwrap();
        }
        if self.state.now() == crimocracy::core::time::SimTime::MAX {
            writeln!(
                out,
                "Simulation clock limit reached; no further minutes can advance."
            )
            .unwrap();
        }
        for report in self
            .state
            .reports()
            .reports_for(self.player)
            .rev()
            .take(new_reports.min(6))
        {
            writeln!(out, "  report {}: {}", report.id().raw(), report.title()).unwrap();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crimocracy::core::time::SimTime;

    #[test]
    fn unfunded_payroll_interrupts_once_per_boundary_and_respects_preference() {
        use crimocracy::world::world_system::{
            designate_player_organization, insert_character, insert_organization,
        };
        use crimocracy::world::{
            AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind,
        };
        let registry = crimocracy::build_registry();
        let mut state = AppState::new(17);
        let player = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Unfunded crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap();
        designate_player_organization(&mut state, player).unwrap();
        insert_character(
            &mut state,
            CharacterDraft {
                name: "Boss".to_owned(),
                organization: Some(player),
                supervisor: None,
                autonomy: AutonomyLevel::Tight,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        let mut session = Session {
            registry: &registry,
            state,
            player,
            seeds: EvaluationSeeds::new(17, 17),
            district: "Test ward".to_owned(),
            district_variant: "TEST",
        };
        assert!(
            session
                .brief()
                .contains("covered from current treasury: 0 cents")
        );
        let first = session.execute("advance 2d").unwrap().0;
        assert!(first.contains("Paused: payroll ran short"), "{first}");
        assert_eq!(session.state.now().as_minutes(), DAY_MINUTES);
        assert!(!session.execute("advance 1m").unwrap().0.contains("Paused:"));
        let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
        assert!(session.execute("attention exception maybe").is_err());
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
        );
        session.execute("attention exception off").unwrap();
        session.state =
            restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
        assert!(
            session
                .execute("attention")
                .unwrap()
                .0
                .contains("Exception auto-pause: off")
        );
        assert!(!session.execute("advance 2d").unwrap().0.contains("Paused:"));
        assert_eq!(session.state.now().as_minutes(), 3 * DAY_MINUTES + 1);
    }

    #[test]
    fn first_enterprise_loss_interrupts_but_repeat_losses_do_not() {
        use crimocracy::world::world_system::insert_neighborhood;
        use crimocracy::world::{
            NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
            NeighborhoodProfile, Rating,
        };
        let registry = crimocracy::build_registry();
        let mut session =
            Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small)
                .unwrap();
        let rating = |n| Rating::try_new(n).unwrap();
        let district = insert_neighborhood(
            &mut session.state,
            NeighborhoodDraft {
                name: "Unprofitable ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(0),
                        commercial_activity: rating(0),
                        illicit_demand: rating(0),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(100),
                    },
                },
            },
        )
        .unwrap();
        let manager = session
            .state
            .world()
            .characters_in_organization(session.player)
            .find(|p| {
                p.capability(crimocracy::world::CapabilityKind::Management)
                    .is_none()
                    && p.supervisor().is_some()
                    && session
                        .state
                        .delegation()
                        .active_for_manager(p.id())
                        .is_none()
            })
            .unwrap()
            .id();
        session
            .execute(&format!(
                "delegate {} district-{}",
                manager.raw(),
                district.raw()
            ))
            .unwrap();
        let cash = session
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(session.player))
            .filter(|account| account.kind() == AccountKind::StreetCash)
            .max_by_key(|account| account.balance().cents())
            .unwrap()
            .id();
        session.execute("open-account settlement").unwrap();
        let settlement = session
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(session.player))
            .filter(|account| account.kind() == AccountKind::Settlement)
            .map(|account| account.id())
            .max()
            .unwrap();
        session
            .execute(&format!(
                "start-racket protection district-{} {} {} manager=char-{}",
                district.raw(),
                cash.raw(),
                settlement.raw(),
                manager.raw()
            ))
            .unwrap();
        let first = session.execute("advance 2d").unwrap().0;
        assert!(first.contains("entered a losing cycle"), "{first}");
        assert_eq!(session.state.now().as_minutes(), DAY_MINUTES);
        let second = session.execute("advance 1d").unwrap().0;
        assert!(!second.contains("entered a losing cycle"), "{second}");
        assert_eq!(session.state.now().as_minutes(), 2 * DAY_MINUTES);

        let racket = session
            .state
            .enterprises()
            .enterprises_for_organization(session.player)
            .find(|racket| racket.location() == EnterpriseLocation::Neighborhood(district))
            .unwrap();
        let id = racket.id();
        if racket.status() == EnterpriseStatus::Active {
            crimocracy::enterprises::enterprise_execution::validate_suspend_enterprise(
                &session.state,
                id,
            )
            .unwrap()
            .commit(&mut session.state)
            .unwrap();
        }
        crimocracy::enterprises::enterprise_execution::validate_resume_enterprise(
            &registry,
            &session.state,
            id,
        )
        .unwrap()
        .commit(&mut session.state)
        .unwrap();
        session.state =
            restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
        let reopened = session.execute("advance 2d").unwrap().0;
        assert!(reopened.contains("entered a losing cycle"), "{reopened}");
        assert_eq!(session.state.now().as_minutes(), 3 * DAY_MINUTES);
    }

    #[test]
    fn advancement_clamps_to_the_final_representable_minute() {
        assert_eq!(bounded_advance_span(SimTime::ZERO, 20).unwrap(), 20);
        assert_eq!(
            bounded_advance_span(SimTime::from_minutes(u64::MAX - 2), 20).unwrap(),
            2
        );
        assert_eq!(
            bounded_advance_span(SimTime::from_minutes(u64::MAX - 5), 2).unwrap(),
            2
        );
        assert!(bounded_advance_span(SimTime::MAX, 1).is_err());
    }
}
