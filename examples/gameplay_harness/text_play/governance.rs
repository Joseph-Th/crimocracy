//! Player governance: standing policies and delegated managerial authority.
//! All writes use the production delegation and world validators, never direct state changes. Every query is restricted to the player's organization.

use super::*;
use crimocracy::world::AutonomyLevel;
use crimocracy::world::world_system::{
    validate_set_character_autonomy, validate_set_character_supervisor,
};

fn parse_policy_setting(name: &str, choice: &str) -> Result<PolicySetting, String> {
    match (name, choice) {
        ("recruitment", "approve") => Ok(PolicySetting::IndependentRecruitment(
            ApprovalPolicy::RequireApproval,
        )),
        ("recruitment", "delegate") => Ok(PolicySetting::IndependentRecruitment(
            ApprovalPolicy::Delegated,
        )),
        ("legal", "case") => Ok(PolicySetting::AssociateLegalSupport(
            LegalSupportPolicy::CaseByCase,
        )),
        ("legal", "automatic") => Ok(PolicySetting::AssociateLegalSupport(
            LegalSupportPolicy::Automatic,
        )),
        _ => Err("policies: recruitment <approve|delegate>, legal <case|automatic>".to_owned()),
    }
}

impl Session<'_> {
    fn scope_label(&self, scope: ResponsibilityScope) -> String {
        match scope {
            ResponsibilityScope::Function(function) => format!("{function:?}"),
            ResponsibilityScope::Neighborhood(district) => {
                let name = self
                    .state
                    .world()
                    .get_neighborhood(district)
                    .map_or("unknown district", |district| district.name());
                format!("district-{} {name}", district.raw())
            }
            ResponsibilityScope::Business(business) => {
                let name = self
                    .state
                    .world()
                    .get_business(business)
                    .map_or("unknown business", |business| business.name());
                format!("business-{} {name}", business.raw())
            }
        }
    }

    fn player_scope(&self, raw: &str) -> Result<ResponsibilityScope, String> {
        let function = match raw {
            "enterprise" => Some(ResponsibilityFunction::Enterprise),
            "territory" => Some(ResponsibilityFunction::Territory),
            "operations" => Some(ResponsibilityFunction::Operations),
            "intelligence" => Some(ResponsibilityFunction::Intelligence),
            "finance" => Some(ResponsibilityFunction::Finance),
            "legal" => Some(ResponsibilityFunction::Legal),
            "political" => Some(ResponsibilityFunction::Political),
            "personnel" => Some(ResponsibilityFunction::Personnel),
            _ => None,
        };
        if let Some(function) = function {
            return Ok(ResponsibilityScope::Function(function));
        }
        if raw.starts_with("neighborhood-") || raw.starts_with("district-") {
            let digits = raw.strip_prefix("district-").unwrap_or(raw);
            let id = parse_number(digits, "neighborhood")?;
            return self
                .state
                .world()
                .neighborhoods()
                .find(|district| district.id().raw() == id)
                .map(|district| ResponsibilityScope::Neighborhood(district.id()))
                .ok_or_else(|| format!("district {id} is not in the public city geography"));
        }
        if raw.starts_with("business-") {
            let id = parse_number(raw, "business")?;
            return self
                .state
                .world()
                .businesses_owned_by_organization(self.player)
                .find(|business| business.id().raw() == id)
                .map(|business| ResponsibilityScope::Business(business.id()))
                .ok_or_else(|| format!("business {id} is not an owned establishment"));
        }
        Err("scope must be enterprise, territory, operations, intelligence, finance, legal, political, personnel, district-N or business-N".to_owned())
    }

    fn player_mandate(&self, raw: &str) -> Result<&crimocracy::delegation::MandateRecord, String> {
        let id = parse_number(raw, "mandate")?;
        self.state
            .world()
            .characters_in_organization(self.player)
            .filter_map(|member| self.state.delegation().active_for_manager(member.id()))
            .find(|mandate| mandate.id().raw() == id && mandate.organization() == self.player)
            .ok_or_else(|| format!("no active player-owned mandate {id}"))
    }

    pub(super) fn execute_governance(&mut self, words: &[&str]) -> Result<String, String> {
        let mut out = String::new();
        match words[0] {
            "policies" => {
                let organization = self
                    .state
                    .world()
                    .get_organization(self.player)
                    .ok_or("player organization disappeared")?;
                for kind in ALL_POLICY_KINDS {
                    let setting = organization
                        .policy(kind)
                        .ok_or("organization policy missing")?;
                    writeln!(out, "{kind:?}: {setting:?}").unwrap();
                }
            }
            "policy" => {
                let setting = parse_policy_setting(words[1], words[2])?;
                validate_set_policy(self.registry, &self.state, self.player, setting)
                    .map_err(|error| error.to_string())?
                    .commit(self.registry, &mut self.state)
                    .map_err(|error| error.to_string())?;
                writeln!(out, "Organization policy updated: {setting:?}.").unwrap();
            }
            "autonomy" | "supervise" => {
                let raw = parse_number(words[1], "char")?;
                let member = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|member| member.id().raw() == raw)
                    .ok_or("no current member with that ID")?;
                let id = member.id();
                if words[0] == "autonomy" {
                    let autonomy = match words[2] {
                        "tight" => AutonomyLevel::Tight,
                        "guided" => AutonomyLevel::Guided,
                        "delegated" => AutonomyLevel::Delegated,
                        "broad" => AutonomyLevel::Broad,
                        _ => {
                            return Err(
                                "autonomy <member-id> <tight|guided|delegated|broad>".to_owned()
                            );
                        }
                    };
                    validate_set_character_autonomy(&self.state, self.player, id, autonomy)
                        .map_err(|error| error.to_string())?
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                    writeln!(out, "char-{raw} autonomy: {autonomy:?}.").unwrap();
                } else {
                    if member.supervisor().is_none() {
                        return Err(
                            "cannot subordinate the organization's executive through this command"
                                .to_owned(),
                        );
                    }
                    let supervisor_raw = parse_number(words[2], "char")?;
                    let supervisor = self
                        .state
                        .world()
                        .characters_in_organization(self.player)
                        .find(|member| member.id().raw() == supervisor_raw)
                        .ok_or("supervisor must be a current member")?
                        .id();
                    validate_set_character_supervisor(&self.state, id, Some(supervisor))
                        .map_err(|error| error.to_string())?
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                    writeln!(out, "char-{raw} now reports to char-{supervisor_raw}.").unwrap();
                }
            }
            "mandates" => {
                for member in self.state.world().characters_in_organization(self.player) {
                    if let Some(record) = self.state.delegation().active_for_manager(member.id()) {
                        if record.organization() != self.player {
                            continue;
                        }
                        let scopes = record
                            .scopes()
                            .iter()
                            .map(|scope| self.scope_label(*scope))
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(
                            out,
                            "{} {} (char-{}) | scopes: {} | autonomy {:?}",
                            record.id().raw(),
                            member.name(),
                            member.id().raw(),
                            scopes,
                            member.autonomy()
                        )
                        .unwrap();
                        if !record.standing_orders().is_empty() {
                            writeln!(out, "  Standing orders: {:?}", record.standing_orders())
                                .unwrap();
                        }
                        if let Some(budget) = record.budget() {
                            writeln!(
                                out,
                                "  Budget: {:?} | {} cents | account {}",
                                budget.period,
                                budget.limit.cents(),
                                budget.funding_account.raw()
                            )
                            .unwrap();
                        }
                    }
                }
                if out.is_empty() {
                    out.push_str("No active managerial mandates.");
                }
            }
            "delegate" => {
                let raw = parse_number(words[1], "char")?;
                let manager = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|member| member.id().raw() == raw)
                    .ok_or("manager is not a current organization member")?
                    .id();
                let scope = self.player_scope(words[2])?;
                let mandate = validate_assign_mandate(
                    &self.state,
                    MandateDraft {
                        organization: self.player,
                        manager,
                        scopes: BTreeSet::from([scope]),
                        standing_orders: BTreeMap::new(),
                        budget: None,
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                let label = self.scope_label(scope);
                writeln!(
                    out,
                    "Delegated {label} to char-{raw} as mandate {}.",
                    mandate.raw()
                )
                .unwrap();
            }
            "mandate" => {
                let record = self.player_mandate(words[1])?;
                let id = record.id();
                match words[2] {
                    "revoke" => {
                        if words.len() != 3 {
                            return Err("mandate <id> revoke".to_owned());
                        }
                        validate_revoke_mandate(&self.state, id)
                            .map_err(|error| error.to_string())?
                            .commit(&mut self.state)
                            .map_err(|error| error.to_string())?;
                        writeln!(out, "Mandate {} revoked.", id.raw()).unwrap();
                    }
                    "add" | "remove" => {
                        if words.len() != 4 {
                            return Err("mandate <id> <add|remove> <scope>".to_owned());
                        }
                        let scope = self.player_scope(words[3])?;
                        let mut scopes = record.scopes().clone();
                        if words[2] == "add" {
                            if !scopes.insert(scope) {
                                return Err("scope is already delegated".to_owned());
                            }
                        } else if !scopes.remove(&scope) {
                            return Err("scope is not delegated by this mandate".to_owned());
                        }
                        let draft = MandateRevisionDraft {
                            scopes,
                            standing_orders: record.standing_orders().clone(),
                            budget: record.budget(),
                        };
                        validate_revise_mandate(&self.state, id, draft)
                            .map_err(|error| error.to_string())?
                            .commit(&mut self.state)
                            .map_err(|error| error.to_string())?;
                        let label = self.scope_label(scope);
                        writeln!(out, "Mandate {} revised: {} {label}.", id.raw(), words[2])
                            .unwrap();
                    }
                    "order" => {
                        if words.len() != 5 {
                            return Err("mandate <id> order <recruitment|legal> <setting|inherit>"
                                .to_owned());
                        }
                        let kind = match words[3] {
                            "recruitment" => PolicyKind::IndependentRecruitment,
                            "legal" => PolicyKind::AssociateLegalSupport,
                            _ => {
                                return Err("mandate order must be recruitment or legal".to_owned());
                            }
                        };
                        let mut standing_orders = record.standing_orders().clone();
                        if words[4] == "inherit" {
                            if standing_orders.remove(&kind).is_none() {
                                return Err(
                                    "mandate already inherits this organization policy".to_owned()
                                );
                            }
                        } else {
                            let setting = parse_policy_setting(words[3], words[4])?;
                            standing_orders.insert(kind, setting);
                        }
                        let draft = MandateRevisionDraft {
                            scopes: record.scopes().clone(),
                            standing_orders,
                            budget: record.budget(),
                        };
                        validate_revise_mandate(&self.state, id, draft)
                            .map_err(|error| error.to_string())?
                            .commit(&mut self.state)
                            .map_err(|error| error.to_string())?;
                        writeln!(out, "Mandate {} standing orders updated.", id.raw()).unwrap();
                    }
                    "budget" => {
                        let budget = if words.len() == 4 && words[3] == "clear" {
                            None
                        } else if words.len() == 6 {
                            let period = match words[3] {
                                "daily" => BudgetPeriod::Daily,
                                "weekly" => BudgetPeriod::Weekly,
                                _ => return Err("budget period must be daily or weekly".to_owned()),
                            };
                            let raw = parse_number(words[4], "account")?;
                            let account = self
                                .state
                                .finance()
                                .accounts_for(FinancialOwner::Organization(self.player))
                                .find(|account| account.id().raw() == raw)
                                .ok_or("budget account is not owned by your organization")?;
                            if account.kind() != AccountKind::AccountedFunds {
                                return Err("mandate budgets require accounted funds".to_owned());
                            }
                            let cents = words[5]
                                .parse::<i64>()
                                .ok()
                                .filter(|value| *value >= 0)
                                .ok_or("budget must be a nonnegative number of cents")?;
                            Some(BudgetAuthority {
                                funding_account: account.id(),
                                limit: Money::from_cents(cents),
                                period,
                            })
                        } else {
                            return Err(
                                "mandate <id> budget clear | <daily|weekly> <account-id> <cents>"
                                    .to_owned(),
                            );
                        };
                        let draft = MandateRevisionDraft {
                            scopes: record.scopes().clone(),
                            standing_orders: record.standing_orders().clone(),
                            budget,
                        };
                        validate_revise_mandate(&self.state, id, draft)
                            .map_err(|error| error.to_string())?
                            .commit(&mut self.state)
                            .map_err(|error| error.to_string())?;
                        writeln!(out, "Mandate {} budget updated.", id.raw()).unwrap();
                    }
                    _ => return Err("mandate <id> <add|remove|order|budget|revoke> ...".to_owned()),
                }
            }
            _ => return Err("unknown governance command".to_owned()),
        }
        Ok(out)
    }
}
