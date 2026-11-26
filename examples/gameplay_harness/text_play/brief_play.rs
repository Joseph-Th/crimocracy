//! Read-only executive briefing for the player's current organization.

use super::*;

impl Session<'_> {
    pub(super) fn brief(&self) -> String {
        if let Some(event) = self.state.campaign_victory()
            && let crimocracy::history::HistoryEventKind::CampaignVictory { path, .. } =
                event.kind()
        {
            return format!(
                "Campaign over: victory ({}). {}. Review the final reports.",
                path.label(),
                event.summary()
            );
        }
        if self.state.player_organization_defeated() {
            return "Campaign over: your organization dissolved. Review the final reports."
                .to_owned();
        }
        let state = &self.state;
        let player = self.player;
        let name = state
            .world()
            .get_organization(player)
            .map_or("Your organization", |o| o.name());
        let pending = state.decisions().pending_for_recipient(player).count();
        let active = state
            .operations()
            .operations_for_organization(player)
            .filter(|op| {
                !matches!(
                    op.status(),
                    OperationStatus::Completed | OperationStatus::Aborted
                )
            })
            .count();
        let leads = self
            .known_opportunities()
            .into_iter()
            .filter(|opp| opp.status() == OpportunityStatus::Open)
            .count();
        let members = state.world().characters_in_organization(player).count();
        let venues = state
            .world()
            .businesses_owned_by_organization(player)
            .count();
        let public_prominence =
            crimocracy::world::organization_pressure::public_prominence_tier(state, player);
        let suspended_venues = state
            .world()
            .businesses_owned_by_organization(player)
            .filter(|business| {
                state
                    .economy()
                    .get_business_economy(business.id())
                    .is_some_and(|economy| {
                        economy.status() == crimocracy::economy::BusinessOperatingStatus::Suspended
                    })
            })
            .count();
        let overhead_basis_points =
            crimocracy::economy::business_economy_system::portfolio_overhead_basis_points(
                state, player,
            );
        let (surplus, estimated_owner_surplus) = estimate_owner_distribution(
            state
                .world()
                .businesses_owned_by_organization(player)
                .filter(|business| {
                    state
                        .economy()
                        .get_business_economy(business.id())
                        .is_some()
                })
                .map(|business| {
                    crimocracy::economy::business_economy_system::resolve_available_business_profit(
                        state,
                        player,
                        business.id(),
                    )
                    .expect("owned operating books must have a valid capital basis")
                }),
            overhead_basis_points,
        );
        let distribution = if state.automatic_profit_sweeps() {
            "daily"
        } else {
            "manual"
        };
        let mut street = 0_i128;
        let mut concealed = 0_i128;
        let mut clean = 0_i128;
        let mut street_spendable = 0_i128;
        let mut concealed_spendable = 0_i128;
        let mut clean_spendable = 0_i128;
        let mut operating_spendable = 0_i128;
        let mut deficits = 0_i128;
        for account in state
            .finance()
            .accounts_for(FinancialOwner::Organization(player))
        {
            let amount = i128::from(account.balance().cents());
            let spendable = i128::from(account.available_payment_balance().cents());
            match account.kind() {
                AccountKind::StreetCash => {
                    street += amount;
                    street_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::ConcealedCash => {
                    concealed += amount;
                    concealed_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::AccountedFunds => {
                    clean += amount;
                    clean_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::LegitimateOperating => {
                    operating_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::Settlement => {}
            }
        }
        let (payroll, covered) = crimocracy::world::payroll_execution::payroll_funding_snapshot(
            self.registry,
            state,
            player,
        )
        .expect("valid player payroll estimate");
        let payroll = payroll.cents();
        let covered = covered.cents();
        // Snapshot only: every organization-owned liquid account can pay wages, but suspended
        // rackets, future shop earnings, counsel fees and fines make forecasts uncertain.
        let wage_liquidity =
            street_spendable + concealed_spendable + clean_spendable + operating_spendable;
        let wage_runway = if payroll > 0 {
            format!("{} full day(s)", wage_liquidity / i128::from(payroll))
        } else {
            "no current wage obligation".to_owned()
        };
        // Actual owner-settled cycles, not a forecast or an extrapolation of held cash.
        // The truncated opening window uses its real elapsed duration so a new campaign
        // doesn't pretend to have already earned 30 days of income.
        let window_minutes = state.now().as_minutes().min(30 * DAY_MINUTES);
        // Both endpoints are inclusive in the reporting API. Exclude the oldest
        // boundary to count exactly the elapsed minutes.
        let window_start = crimocracy::core::time::SimTime::from_minutes(
            (state.now().as_minutes() - window_minutes)
                .saturating_add(1)
                .min(state.now().as_minutes()),
        );
        let days = window_minutes / DAY_MINUTES;
        let hours = window_minutes % DAY_MINUTES / 60;
        let minutes = window_minutes % 60;
        let business_net = crimocracy::economy::business_reporting::resolve_organization_business_financial_summary(
            state, player, window_start, state.now(),
        ).expect("valid player business financial history").totals.net_cash.cents();
        let racket_net = crimocracy::enterprises::enterprise_reporting::resolve_organization_enterprise_financial_summary(
            state, player, window_start, state.now(),
        ).expect("valid player enterprise financial history").totals.net_cash.cents();
        let avg_daily_net = (i128::from(business_net) + i128::from(racket_net))
            * i128::from(DAY_MINUTES)
            / i128::from(window_minutes.max(DAY_MINUTES));
        let payroll_gap = avg_daily_net - i128::from(payroll);
        let treasury_guidance = if !state.automatic_profit_sweeps()
            && surplus > 0
            && covered < payroll
        {
            "\nEarned storefront cash cannot pay wages until withdrawn. Use 'treasury daily' to distribute before payroll, or 'sweep' to withdraw from one storefront."
        } else {
            ""
        };
        let detained_members = state
            .world()
            .characters_in_organization(player)
            .filter(|member| {
                state
                    .legal()
                    .active_arrest_for_character(member.id())
                    .is_some()
            })
            .count();
        let mut active_rackets = 0;
        let mut suspended_rackets = 0;
        let mut active_associates = 0_u64;
        let mut inactive_associates = 0_u64;
        let mut incremental_labor_wages = 0_u64;
        let mut assigned_captains = BTreeSet::new();
        for racket in state.enterprises().enterprises_for_organization(player) {
            match racket.status() {
                EnterpriseStatus::Active => {
                    active_rackets += 1;
                    active_associates += u64::from(racket.associates());
                    incremental_labor_wages +=
                        u64::from(racket.associates() - racket.kind().baseline_associates())
                            * crimocracy::enterprises::enterprise_execution::ASSOCIATE_ADDITIONAL_DAILY_WAGE
                                .cents() as u64;
                    assigned_captains.insert(racket.manager());
                }
                EnterpriseStatus::Suspended => {
                    suspended_rackets += 1;
                    inactive_associates += u64::from(racket.associates());
                }
                EnterpriseStatus::Retired => {}
            }
        }
        let rackets = active_rackets + suspended_rackets;
        let command = state
            .world()
            .characters_in_organization(player)
            .filter_map(|person| {
                crimocracy::enterprises::enterprise_execution::manager_workload(state, person.id())
            })
            .filter(|load| load.required > 0)
            .collect::<Vec<_>>();
        let command_required: u64 = command.iter().map(|load| u64::from(load.required)).sum();
        let command_available: u64 = command.iter().map(|load| u64::from(load.available)).sum();
        let overstretched_leaders = command
            .iter()
            .filter(|load| load.required > load.available)
            .count();
        let active_captains = assigned_captains.len();
        let command_lines: u64 = command
            .iter()
            .map(|load| u64::from(load.command_lines))
            .sum();
        let latest = state
            .reports()
            .reports_for(player)
            .next_back()
            .map_or("No reports yet".to_owned(), |report| {
                format!("Report {}: {}", report.id().raw(), report.title())
            });
        format!(
            "{name} | {} | {} ({})\nPeople: {members} named | Associates: {active_associates} active, {inactive_associates} suspended | Venues: {venues} ({suspended_venues} suspended) | Rackets: {rackets} ({active_rackets} active, {suspended_rackets} suspended)\nPublic prominence: tier {public_prominence}/6 | a wider commercial and political reach draws rival resistance and greater scrutiny of active rackets\nTreasury: street {street} cents | concealed {concealed} cents | accounted {clean} cents\nSpendable now: street {street_spendable} cents | concealed {concealed_spendable} cents | accounted {clean_spendable} cents | operating {operating_spendable} cents | separate account deficits {deficits} cents\nWithdrawable storefront surplus: {surplus} cents gross | owner net estimate {estimated_owner_surplus} cents after {overhead_basis_points} bp portfolio administration | distribution: {distribution}\nAssigned racket captains: {active_captains} | all-chain command duties: {command_required}/{command_available} available | direct reporting lines: {command_lines} | overloaded leaders: {overstretched_leaders} (see 'people' for individual workloads)\nNamed-member daily payroll: {payroll} cents | covered from current treasury: {covered} cents (before future revenue)
Gross payroll runway: {wage_runway} at current headcount and accessible funds, ignoring new income and other spending{treasury_guidance}
Additional associate wages: {incremental_labor_wages} cents/day through racket operating expenses (baseline workers included in fixed racket costs)\nRealized operating net before owner-distribution overhead (last {days}d {hours}h {minutes}m): legitimate {business_net} cents | rackets {racket_net} cents | averaged {avg_daily_net} cents/day | vs current payroll {payroll_gap:+} cents/day (historical income against today's wages; minimum one-day averaging window; excludes laundering and asset trades)\nDetained staff: {detained_members} | suspended rackets: {suspended_rackets} | suspended storefronts: {suspended_venues} | use 'counsel' and 'recovery' for actionable details\nPending decisions: {pending} | Active operations: {active} | Open leads: {leads}\nLatest: {latest}\nUse 'reports', 'opportunities', 'people', or 'accounts' to investigate.",
            time_label(state.now().as_minutes()),
            self.district,
            self.district_variant,
        )
    }
}
