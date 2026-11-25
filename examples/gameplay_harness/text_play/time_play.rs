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
    fn own_suspended_assets(&self) -> BTreeSet<EntityRef> {
        let storefronts = self
            .state
            .world()
            .businesses_owned_by_organization(self.player)
            .filter_map(|business| {
                self.state
                    .economy()
                    .get_business_economy(business.id())
                    .filter(|books| {
                        books.status() == crimocracy::economy::BusinessOperatingStatus::Suspended
                    })
                    .map(|_| EntityRef::Business(business.id()))
            });
        let rackets = self
            .state
            .enterprises()
            .enterprises_for_organization(self.player)
            .filter(|racket| racket.status() == EnterpriseStatus::Suspended)
            .map(|racket| EntityRef::Enterprise(racket.id()));
        storefronts.chain(rackets).collect()
    }

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
        let mut deferred_contracts = 0_usize;
        let mut deferred_departures = 0_usize;
        // Player assets cannot autonomously reopen or retire during advancement.
        // Compare the endpoints instead of guessing every possible shutdown cause
        // from individual tick fields or rescanning the player's holdings every minute.
        let initially_suspended = uninterrupted.then(|| self.own_suspended_assets());
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
            let own_contract_expirations = tick
                .freelance_contract_expirations
                .iter()
                .filter(|&&freelancer| {
                    self.state
                        .world()
                        .latest_freelance_contract(freelancer)
                        .is_some_and(|term| term.sponsor() == self.player)
                })
                .count();
            let own_payroll_shortfall = tick.payrolls.iter().any(|payroll| {
                payroll.organization() == self.player && payroll.short() > Money::ZERO
            });
            // Membership has already been released by the world owner. Associate each
            // departure with the *original* player payroll rather than inspecting its
            // new unaffiliated state or disclosing rival departures.
            let own_payroll_departures = tick
                .payrolls
                .iter()
                .find(|payroll| payroll.organization() == self.player)
                .map_or(0, |payroll| {
                    tick.payroll_departures
                        .iter()
                        .filter(|id| payroll.underpaid_members().contains(id))
                        .count()
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
                deferred_contracts += own_contract_expirations;
                deferred_departures += own_payroll_departures;
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
            } else if !uninterrupted && own_contract_expirations > 0 {
                reason = Some("a freelance contract expired; review 'people' and 'freelancers'");
            } else if !uninterrupted && pause_exceptions && own_payroll_departures > 0 {
                reason = Some(
                    "a member left after unpaid wages; review 'people', 'treasury' and 'reports'",
                );
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
        if let Some(initially_suspended) = initially_suspended {
            let deferred_shutdowns = self
                .own_suspended_assets()
                .difference(&initially_suspended)
                .count();
            writeln!(
                out,
                "Uninterrupted review: {deferred_custody} custody event(s), {deferred_releases} release(s), {deferred_contracts} freelance contract end(s), {deferred_departures} unpaid-wage departure(s), {deferred_shutdowns} shutdown event(s), {deferred_losses} new loss streak(s), {deferred_payroll} payroll shortfall(s), {deferred_operations} completed/aborted operation minute(s), {deferred_urgent} priority-report minute(s). All events were simulated. Review 'reports', 'people', 'freelancers', 'counsel', 'recovery' and 'accounts'."
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
    fn uninterrupted_advance_counts_custody_shutdown_once() {
        let registry = crimocracy::build_registry();
        let mut session =
            Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small)
                .unwrap();
        let result = session.execute("advance 7d uninterrupted").unwrap().0;
        assert!(result.contains("1 custody event(s)"), "{result}");
        assert!(result.contains("1 release(s)"), "{result}");
        assert!(
            session
                .state
                .enterprises()
                .enterprises_for_organization(session.player)
                .any(|racket| racket.status() == EnterpriseStatus::Suspended)
        );
        assert!(result.contains("1 shutdown event(s)"), "{result}");

        let next = session.execute("advance 1m uninterrupted").unwrap().0;
        assert!(next.contains("0 shutdown event(s)"), "{next}");
    }

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
        assert!(
            session
                .brief()
                .contains("Gross payroll runway: 0 full day(s)"),
            "a completely unfunded crew must have zero current wage runway"
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
    fn uninterrupted_advance_discloses_player_crew_departures_not_just_shortfalls() {
        let registry = crimocracy::build_registry();
        use crimocracy::world::world_system::{
            designate_player_organization, insert_character, insert_neighborhood,
            insert_organization,
        };
        use crimocracy::world::{
            AutonomyLevel, CapabilityKind, CharacterDraft, NeighborhoodDraft,
            NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile,
            OrganizationDraft, OrganizationKind, Rating,
        };
        let mut state = AppState::new(17);
        let player = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Unpaid crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap();
        designate_player_organization(&mut state, player).unwrap();
        let boss = insert_character(
            &mut state,
            CharacterDraft {
                name: "Leader".to_owned(),
                organization: Some(player),
                supervisor: None,
                autonomy: AutonomyLevel::Tight,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        let captain = insert_character(
            &mut state,
            CharacterDraft {
                name: "Associate".to_owned(),
                organization: Some(player),
                supervisor: Some(boss),
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Management,
                    Rating::try_new(90).unwrap(),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        // A second executive can keep the unfunded organization alive after the
        // subordinate walks away; this distinguishes a departure alert from the
        // higher-priority terminal dissolution of the entire organization.
        insert_character(
            &mut state,
            CharacterDraft {
                name: "Second leader".to_owned(),
                organization: Some(player),
                supervisor: None,
                autonomy: AutonomyLevel::Tight,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        let rating = Rating::try_new(50).unwrap();
        let district = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Test ward".into(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating,
                        commercial_activity: rating,
                        illicit_demand: rating,
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating,
                    },
                },
            },
        )
        .unwrap();
        let scope = ResponsibilityScope::Neighborhood(district);
        let mandate = validate_assign_mandate(
            &state,
            MandateDraft {
                organization: player,
                manager: captain,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        let cash_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(player),
                kind: AccountKind::StreetCash,
            },
        )
        .unwrap();
        let settlement_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(player),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        let racket = validate_establish_enterprise(
            &registry,
            &state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization: player,
                authority: MandateAuthority {
                    mandate,
                    manager: captain,
                    scope,
                },
                location: EnterpriseLocation::Neighborhood(district),
                supporting_businesses: BTreeSet::new(),
                cash_account,
                settlement_account,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        validate_suspend_enterprise(&state, racket)
            .unwrap()
            .commit(&mut state)
            .unwrap();

        // All lead-up minutes use the production pipeline. The fifth wage boundary
        // must reach the text client with a real departure event.
        for _ in 0..5 * DAY_MINUTES - 1 {
            let tick = run_tick(&registry, &mut state).unwrap();
            assert!(tick.payroll_departures.is_empty());
        }
        let mut normal = Session {
            registry: &registry,
            state: state.clone(),
            player,
            seeds: EvaluationSeeds::new(17, 17),
            district: "Test ward".to_owned(),
            district_variant: "TEST",
        };
        let paused = normal.execute("advance 1m").unwrap().0;
        assert!(
            paused.contains("Paused: a member left after unpaid wages"),
            "{paused}"
        );
        let mut session = Session {
            registry: &registry,
            state,
            player,
            seeds: EvaluationSeeds::new(17, 17),
            district: "Test ward".to_owned(),
            district_variant: "TEST",
        };
        let review = session.execute("advance 1m uninterrupted").unwrap().0;
        let prior = review
            .split("unpaid-wage departure(s)")
            .next()
            .expect("review must disclose crew departures");
        let departures: usize = prior
            .split_whitespace()
            .last()
            .expect("review must include a departure count")
            .parse()
            .expect("departure count must be numeric");
        assert!(
            departures > 0,
            "crew attrition must appear in review: {review}"
        );
        assert!(review.contains("payroll shortfall(s)"), "{review}");
        assert_eq!(
            departures, 1,
            "the unpaid associate should depart: {review}"
        );
        assert!(!session.state.player_organization_defeated());
        assert_ne!(
            session
                .state
                .world()
                .get_character(captain)
                .unwrap()
                .organization(),
            Some(player)
        );
        let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
        let recovery = session.execute("recovery").unwrap().0;
        assert!(
            recovery.contains(&format!("char-{}, former member", captain.raw())),
            "{recovery}"
        );
        assert!(recovery.contains("Resumption blocked:"), "{recovery}");
        assert!(!recovery.contains("Can resume manually:"), "{recovery}");
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
        );
        let restored =
            restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
        crimocracy::core::invariants::validate_state(&restored).unwrap();
        session.state = restored;
        assert_eq!(session.execute("recovery").unwrap().0, recovery);
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
