//! Acquiring a business as an organizational asset through a listing or a negotiated offer.
//!
//! The canonical purchase path: an organization buys a business outright at its authored
//! kind price, paid in full from accounted funds. Dirty street cash cannot become a
//! storefront — legitimate expansion is gated on laundering throughput, so the money
//! loop closes: illicit proceeds are laundered into accounted wealth, and accounted
//! wealth converts into earning capacity (and, for cash-intensive fronts, more
//! laundering capacity). Purchase consideration leaves the buyer's spendable control:
//! the acquired business's settlement account represents independent sellers, while a negotiated
//! organization-owned sale credits the actual seller's accounted treasury. Neither capitalizes
//! the asset's operating cash.
//!
//! Commit composes three canonical paths in one validated step: world ownership
//! transfer, business-economy establishment when the target has never operated (or
//! restart under the new owner when books already exist), and a balanced ledger
//! payment. Every fallible condition is validated before any mutation; the records
//! constructed during commit reference freshly reserved accounts whose kinds and owners
//! are guaranteed by construction.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, FinancialAccountId, IdExhaustionError, IdKind, OpportunityId, OrganizationId,
};
use crate::core::state::AppState;
use crate::economy::{BusinessEconomyDraft, business_economy_system};
use crate::finance::finance_system::{
    FinanceError, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::helpers::format_money_cents;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::OperationBusinessTargetOwnership;
use crate::opportunities::OpportunityStatus;
use crate::opportunities::opportunity_system::{OpportunityError, validate_dismiss_opportunity};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::WorldError;
use crate::world::world_system::validate_transfer_business_ownership;
use crate::world::{BusinessFunction, BusinessOwner, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

mod autonomous;
pub(crate) use autonomous::apply_due_autonomous_acquisitions;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessAcquisitionError {
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business {business} is not independently owned; it is owned by {owner:?}")]
    NotIndependentlyOwned {
        business: BusinessId,
        owner: BusinessOwner,
    },
    #[error("funding account {0} does not exist")]
    MissingFundingAccount(FinancialAccountId),
    #[error("funding account {account} is not owned by organization {organization}")]
    FundingAccountOwnerMismatch {
        account: FinancialAccountId,
        organization: OrganizationId,
    },
    #[error(
        "funding account {0} must hold accounted funds: dirty street cash cannot buy legitimacy"
    )]
    InvalidFundingAccountKind(FinancialAccountId),
    #[error(
        "accounted funds {available_cents} cannot cover the {price_cents}-cent acquisition price"
    )]
    InsufficientFunds {
        available_cents: i64,
        price_cents: i64,
    },
    #[error("business acquisition must name at least one accounted-funds account")]
    NoFundingAccounts,
    #[error(
        "the owner declined your acquisition offer; increase the price or consider another property"
    )]
    OfferDeclined,
    #[error("an acquisition offer must be positive")]
    InvalidOffer,
    #[error(transparent)]
    World(#[from] WorldError),
    #[error(transparent)]
    Economy(#[from] business_economy_system::BusinessEconomyError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    Opportunity(#[from] OpportunityError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Debug)]
pub struct BusinessAcquisitionDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    /// Organization-owned accounted-funds accounts permitted to fund the purchase. The ledger
    /// records the exact deterministic debit allocation, so the acquisition itself stores no
    /// duplicate payment-source fact.
    pub funding_accounts: BTreeSet<FinancialAccountId>,
}

/// What one committed acquisition actually did, quoted from production state rather than
/// recomputed by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquiredBusiness {
    pub business: BusinessId,
    pub price: Money,
    /// True when commit also opened the acquired business's first operating economy.
    pub established_economy: bool,
    /// Open foreign-only leads rendered moot by the buyer's new ownership.
    pub dismissed_opportunities: Vec<OpportunityId>,
}

#[derive(Debug)]
pub struct ValidatedBusinessAcquisition {
    /// Authored cycle duration for the economy commit opens; captured at validation time.
    economy_cycle_duration: crate::core::time::SimDuration,
    funding_accounts: BTreeSet<FinancialAccountId>,
    price: Money,
    business: BusinessId,
    business_name: String,
    organization: OrganizationId,
    negotiated_seller: Option<(OrganizationId, u32)>,
}

impl ValidatedBusinessAcquisition {
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<AcquiredBusiness, BusinessAcquisitionError> {
        // ---- Phase 1: re-validate every fallible condition against live state. --------
        // A validated token can be held across intervening mutations, so solvency,
        // ownership, and hosting conflicts are re-checked before anything mutates.
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessAcquisitionError::MissingBusiness(self.business))?;
        if let Some((seller, version)) = self.negotiated_seller {
            if business_record.version() != version
                || eligible_offer_seller(
                    registry,
                    state,
                    self.organization,
                    self.business,
                    self.price,
                )? != seller
            {
                return Err(BusinessAcquisitionError::OfferDeclined);
            }
        } else if business_record.owner() != BusinessOwner::Independent {
            return Err(BusinessAcquisitionError::NotIndependentlyOwned {
                business: self.business,
                owner: business_record.owner(),
            });
        }
        validate_funding_accounts(state, self.organization, &self.funding_accounts, self.price)?;
        // Hosting conflicts (active enterprise venues/supporters) reject through the same
        // canonical read the ownership transfer enforces. Keep the validated token so commit
        // does not create a second validation path later in the composite operation.
        let transfer = validate_transfer_business_ownership(
            state,
            self.business,
            BusinessOwner::Organization(self.organization),
        )?;
        let existing_economy = state.economy.get_business_economy(self.business);
        // The establishment/restart decision follows live state, not the validation-time
        // snapshot: a token held across someone else's establishment adopts those books instead
        // of colliding. Every existing economy restarts from the title-transfer instant so the
        // buyer cannot inherit a nearly due seller cycle or the seller's chronic-loss streak.
        let establish_now = existing_economy.is_none();

        // ---- Phase 2: plan fresh books and pre-validate every remaining leg. ---------
        // Fresh account IDs are predicted without consuming allocator state. The payment token
        // owns that plan and opens the accounts only when the whole acquisition is ready to
        // commit, so no rollback path exists and every rejection is truly state-neutral.
        let mut establishment = None;
        let payment = if let Some((seller, _)) = self.negotiated_seller {
            // The seller actually receives spendable purchase consideration. Select a viable
            // account deterministically or open one atomically with the transfer.
            let destination = state
                .finance
                .accounts_for(FinancialOwner::Organization(seller))
                .find(|account| {
                    account.kind() == AccountKind::AccountedFunds
                        && account.version() < u32::MAX
                        && account.balance().checked_add(self.price).is_some()
                })
                .map(|account| account.id());
            if let Some(destination) = destination {
                validate_record_transaction(
                    state,
                    acquisition_payment_draft(
                        state,
                        &self.funding_accounts,
                        destination,
                        self.price,
                        &self.business_name,
                    ),
                )?
            } else {
                let openings = validate_open_accounts(
                    state,
                    vec![FinancialAccountDraft {
                        owner: FinancialOwner::Organization(seller),
                        kind: AccountKind::AccountedFunds,
                    }],
                )?;
                let destination = openings
                    .account_id(0)
                    .expect("seller account opening must expose its ID");
                validate_record_transaction_with_openings(
                    state,
                    openings,
                    acquisition_payment_draft(
                        state,
                        &self.funding_accounts,
                        destination,
                        self.price,
                        &self.business_name,
                    ),
                )?
            }
        } else if let Some(economy) = existing_economy {
            validate_record_transaction(
                state,
                acquisition_payment_draft(
                    state,
                    &self.funding_accounts,
                    economy.settlement_account(),
                    self.price,
                    &self.business_name,
                ),
            )?
        } else {
            let openings = validate_open_accounts(
                state,
                vec![
                    FinancialAccountDraft {
                        owner: FinancialOwner::Business(self.business),
                        kind: AccountKind::LegitimateOperating,
                    },
                    FinancialAccountDraft {
                        owner: FinancialOwner::Business(self.business),
                        kind: AccountKind::Settlement,
                    },
                ],
            )?;
            let operating_account = openings
                .account_id(0)
                .expect("two-account acquisition plan must expose its operating account");
            let settlement_account = openings
                .account_id(1)
                .expect("two-account acquisition plan must expose its settlement account");
            establishment = Some(
                business_economy_system::validate_composed_business_economy_establishment(
                    state,
                    BusinessEconomyDraft {
                        business: self.business,
                        operating_account,
                        settlement_account,
                    },
                    self.economy_cycle_duration,
                    &openings,
                )?,
            );
            validate_record_transaction_with_openings(
                state,
                openings,
                acquisition_payment_draft(
                    state,
                    &self.funding_accounts,
                    settlement_account,
                    self.price,
                    &self.business_name,
                ),
            )?
        };
        let restart = if existing_economy.is_some() {
            Some(business_economy_system::validate_acquisition_restart(
                state,
                self.business,
                self.economy_cycle_duration,
            )?)
        } else {
            None
        };
        // Only the buyer learns that its own foreign-target leads have become redundant.
        // Keep multi-target leads open when an alternative target may still be actionable.
        // Prepare the current set at commit, not when the purchase was first validated.
        let dismissals = state
            .opportunities()
            .opportunities_for_organization(self.organization)
            .filter(|record| {
                record.status() == OpportunityStatus::Open
                    && record
                        .valid_until()
                        .is_none_or(|deadline| state.now() < deadline)
                    && record
                        .context()
                        .operation_kind()
                        .business_target_ownership()
                        == Some(OperationBusinessTargetOwnership::Foreign)
                    && record.context().targets().len() == 1
                    && record
                        .context()
                        .targets()
                        .contains(&EntityRef::Business(self.business))
            })
            .map(|record| {
                Ok((
                    record.id(),
                    validate_dismiss_opportunity(registry, state, record.id())?,
                ))
            })
            .collect::<Result<Vec<_>, OpportunityError>>()?;
        let announcement = validate_record_report(
            state,
            ReportDraft {
                recipient: self.organization,
                kind: ReportKind::Financial,
                title: "Business acquisition".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: format!(
                        "The organization purchased {} outright for {}, paid in full from accounted funds.",
                        self.business_name,
                        format_money_cents(self.price.cents())
                    ),
                    sources: Vec::new(),
                    entities: std::collections::BTreeSet::from([EntityRef::Business(
                        self.business,
                    )]),
                    decision: None,
                }],
            },
        )?;

        // Listing availability is public, but buyers, balances and private enterprise
        // dependencies are not. Notify the player that a listed property has left the market
        // without identifying the purchaser or its financing.
        let market_notice = state
            .player_organization()
            .filter(|player| self.negotiated_seller.is_none() && *player != self.organization)
            .map(|recipient| {
                validate_record_report(
                    state,
                    ReportDraft {
                        recipient,
                        kind: ReportKind::Financial,
                        title: "Property market".to_owned(),
                        entries: vec![ReportEntry {
                            attention: AttentionClass::Notable,
                            summary: format!(
                                "{} has been sold and is no longer available for direct purchase.",
                                self.business_name
                            ),
                            sources: Vec::new(),
                            entities: BTreeSet::from([EntityRef::Business(self.business)]),
                            decision: None,
                        }],
                    },
                )
            })
            .transpose()?;

        let seller_notice = self.negotiated_seller.map(|(seller, _)| {
            validate_record_report(state, ReportDraft {
                recipient: seller,
                kind: ReportKind::Financial,
                title: "Business sale".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: format!("The organization accepted an offer of {} for {} and transferred ownership.",
                        format_money_cents(self.price.cents()), self.business_name),
                    sources: Vec::new(),
                    entities: BTreeSet::from([EntityRef::Business(self.business)]),
                    decision: None,
                }],
            })
        }).transpose()?;

        // Property transfers are institutional business: commercial registers track the new
        // owner, political staff see title changes, and labor officials see union-access
        // establishments change hands. Only those institutions hold the evidence; contacts
        // must still disclose it through their normal relationship-gated channel.
        let institutional_notices =
            plan_institutional_sale_observations(state, self.organization, self.business)?;

        // ---- Phase 3: reserve the complete persistent-ID budget, then commit. --------
        // No mutation occurs before this aggregate preflight. From this point onward each
        // fallible canonical commit is guarded by dependencies that this function alone controls.
        let mut id_budget = payment.id_budget();
        id_budget.extend([(IdKind::BusinessOwnershipChange, 1), (IdKind::Report, 1)]);
        id_budget.extend(std::iter::repeat_n((IdKind::Report, 1), dismissals.len()));
        if market_notice.is_some() {
            id_budget.push((IdKind::Report, 1));
        }
        if seller_notice.is_some() {
            id_budget.push((IdKind::Report, 1));
        }
        id_budget.extend(std::iter::repeat_n(
            (IdKind::Information, 1),
            institutional_notices.len(),
        ));
        state.ids.reserve_many(&id_budget)?;
        transfer
            .commit(state)
            .expect("a just-revalidated ownership transfer must commit atomically");
        payment
            .commit(state)
            .expect("a payment pre-validated against untouched account versions must commit");
        if let Some(establishment) = establishment {
            establishment.commit_after_preflight(state);
        } else if let Some(restart) = restart {
            restart
                .commit(state)
                .expect("prevalidated acquisition restart must remain current during commit");
        }
        announcement
            .commit(state)
            .expect("a validated acquisition report about live entities must commit");
        if let Some(notice) = market_notice {
            notice
                .commit(state)
                .expect("prevalidated public market notice must remain current");
        }
        if let Some(notice) = seller_notice {
            notice
                .commit(state)
                .expect("prevalidated seller notice must remain current");
        }
        for notice in institutional_notices {
            notice
                .commit(state)
                .expect("prevalidated sale observation must remain current");
        }
        let dismissed_opportunities = dismissals
            .into_iter()
            .map(|(id, dismissal)| {
                dismissal
                    .commit(state)
                    .expect("prevalidated acquisition dismissal must remain current");
                id
            })
            .collect();
        Ok(AcquiredBusiness {
            business: self.business,
            price: self.price,
            established_economy: establish_now,
            dismissed_opportunities,
        })
    }
}

fn plan_institutional_sale_observations(
    state: &AppState,
    buyer: OrganizationId,
    property: BusinessId,
) -> Result<Vec<ValidatedInformation>, IntelligenceError> {
    let business = state
        .world
        .get_business(property)
        .expect("validated property");
    let buyer_name = state
        .world
        .get_organization(buyer)
        .expect("validated buyer")
        .name();
    let mut observations = Vec::new();
    for institution in state.world.organizations() {
        let (topic, description) = match institution.kind() {
            OrganizationKind::Political => {
                (InformationTopic::MarketAccess, "Property registration")
            }
            OrganizationKind::Commercial => (
                InformationTopic::FinancialPerformance,
                "Commercial ownership update",
            ),
            OrganizationKind::Labor
                if business
                    .functions()
                    .contains(&BusinessFunction::UnionAccess) =>
            {
                (
                    InformationTopic::FinancialPerformance,
                    "Union workplace ownership update",
                )
            }
            OrganizationKind::Criminal
            | OrganizationKind::LawEnforcement
            | OrganizationKind::LegalAuthority
            | OrganizationKind::LegalServices
            | OrganizationKind::Prosecutor
            | OrganizationKind::Labor
            | OrganizationKind::Civic => continue,
        };
        for official in state
            .world
            .characters_in_organization(institution.id())
            .filter(|official| {
                state
                    .legal()
                    .active_arrest_for_character(official.id())
                    .is_none()
            })
        {
            observations.push(validate_record_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Character(official.id()),
                    source_kind: InformationSourceKind::DirectObservation,
                    topic,
                    source_entity: Some(EntityRef::Organization(institution.id())),
                    subject: EntityRef::Organization(buyer),
                    observed_at: state.now(),
                    reliability: Reliability::DirectAccess,
                    specificity: Specificity::Precise,
                    summary: format!(
                        "{description}: {} acquired {}.",
                        buyer_name,
                        business.name()
                    ),
                },
            )?);
        }
    }
    Ok(observations)
}

fn acquisition_payment_draft(
    state: &AppState,
    funding_accounts: &BTreeSet<FinancialAccountId>,
    seller_settlement_account: FinancialAccountId,
    price: Money,
    business_name: &str,
) -> LedgerTransactionDraft {
    // Spend the largest spendable balance first so one well-funded accounted pocket covers
    // the price alone instead of draining every low-ID dust account. ID breaks exact-balance
    // ties. This matches payroll's largest-first funding order; the debit set is otherwise
    // identical, only the fragmentation differs.
    let mut ordered: Vec<(FinancialAccountId, Money)> = funding_accounts
        .iter()
        .map(|account| {
            let spendable = state
                .finance
                .get_account(*account)
                .expect("validated acquisition funding account must exist")
                .spendable_balance();
            (*account, spendable)
        })
        .filter(|(_, spendable)| *spendable != Money::ZERO)
        .collect();
    ordered.sort_by_key(|(account, spendable)| (std::cmp::Reverse(spendable.cents()), *account));
    let mut postings = Vec::with_capacity(funding_accounts.len() + 1);
    let mut remaining = price;
    for (account, spendable) in ordered {
        if remaining == Money::ZERO {
            break;
        }
        let debit = spendable.min(remaining);
        postings.push(LedgerPosting {
            account,
            amount: debit
                .checked_neg()
                .expect("positive acquisition debit must negate"),
        });
        remaining = remaining
            .checked_sub(debit)
            .expect("acquisition debit cannot exceed remaining price");
    }
    debug_assert_eq!(remaining, Money::ZERO);
    postings.push(LedgerPosting {
        account: seller_settlement_account,
        amount: price,
    });
    LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!("Business acquisition of {business_name}"),
        postings,
        authorization: None,
    }
}

fn validate_funding_accounts(
    state: &AppState,
    organization: OrganizationId,
    funding_accounts: &BTreeSet<FinancialAccountId>,
    price: Money,
) -> Result<(), BusinessAcquisitionError> {
    if funding_accounts.is_empty() {
        return Err(BusinessAcquisitionError::NoFundingAccounts);
    }
    let mut available_cents = 0_i128;
    for account in funding_accounts {
        let funding = state
            .finance
            .get_account(*account)
            .ok_or(BusinessAcquisitionError::MissingFundingAccount(*account))?;
        if funding.owner() != FinancialOwner::Organization(organization) {
            return Err(BusinessAcquisitionError::FundingAccountOwnerMismatch {
                account: *account,
                organization,
            });
        }
        if funding.kind() != AccountKind::AccountedFunds {
            return Err(BusinessAcquisitionError::InvalidFundingAccountKind(
                *account,
            ));
        }
        available_cents = (available_cents + i128::from(funding.spendable_balance().cents()))
            .min(i128::from(price.cents()));
    }
    if available_cents < i128::from(price.cents()) {
        return Err(BusinessAcquisitionError::InsufficientFunds {
            available_cents: i64::try_from(available_cents)
                .expect("available acquisition funding is bounded by price"),
            price_cents: price.cents(),
        });
    }
    Ok(())
}

/// Public offer for an independently owned property that can be transferred to this buyer.
/// This deliberately discloses only listed infrastructure and price, never a rival's owner,
/// ledger, enterprise dependencies, or operating performance. Funds are checked at purchase.
pub fn acquisition_offer<'state>(
    registry: &Registry,
    state: &'state AppState,
    organization: OrganizationId,
    business: BusinessId,
) -> Option<(Money, &'state BTreeSet<crate::world::BusinessFunction>)> {
    let record = state.world.get_business(business)?;
    if record.owner() != BusinessOwner::Independent {
        return None;
    }
    validate_transfer_business_ownership(
        state,
        business,
        BusinessOwner::Organization(organization),
    )
    .ok()?;
    Some((
        registry
            .get_business(record.kind())
            .economics()
            .acquisition_cost(),
        record.functions(),
    ))
}

pub fn validate_acquire_business(
    registry: &Registry,
    state: &AppState,
    draft: BusinessAcquisitionDraft,
) -> Result<ValidatedBusinessAcquisition, BusinessAcquisitionError> {
    if state.world.get_organization(draft.organization).is_none() {
        return Err(BusinessAcquisitionError::MissingOrganization(
            draft.organization,
        ));
    }
    let business_record = state
        .world
        .get_business(draft.business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(draft.business))?;
    if business_record.owner() != BusinessOwner::Independent {
        return Err(BusinessAcquisitionError::NotIndependentlyOwned {
            business: draft.business,
            owner: business_record.owner(),
        });
    }
    let price = registry
        .get_business(business_record.kind())
        .economics()
        .acquisition_cost();
    validate_funding_accounts(state, draft.organization, &draft.funding_accounts, price)?;
    // Ownership conflicts (active enterprise hosts/supports) reject through the canonical
    // transfer path; unchanged ownership is impossible because an Independent owner never
    // equals an Organization owner. Commit re-runs this read against live state, so the
    // token itself carries no transfer snapshot.
    validate_transfer_business_ownership(
        state,
        draft.business,
        BusinessOwner::Organization(draft.organization),
    )?;
    // Existing books do not block a purchase. Acquisition changes the responsible owner, so
    // commit restarts the cycle after ownership transfer: suspended books reopen, active books
    // re-anchor their next settlement, and neither state inherits the seller's loss streak.
    Ok(ValidatedBusinessAcquisition {
        economy_cycle_duration: registry
            .get_business(business_record.kind())
            .economics()
            .cycle(),
        funding_accounts: draft.funding_accounts,
        price,
        business: draft.business,
        business_name: business_record.name().to_owned(),
        organization: draft.organization,
        negotiated_seller: None,
    })
}

/// A voluntary offer for an operating property held by another organization. The seller's
/// decision values the asset's standard acquisition cost plus ninety days of expected
/// positive operating income. The minimum is not an advertised quote, and refusals disclose
/// neither private ownership nor dependency information. A seller will not relinquish an
/// enterprise's infrastructure, including suspended rackets that it could resume.
fn eligible_offer_seller(
    registry: &Registry,
    state: &AppState,
    buyer: OrganizationId,
    business: BusinessId,
    offer: Money,
) -> Result<OrganizationId, BusinessAcquisitionError> {
    if offer <= Money::ZERO {
        return Err(BusinessAcquisitionError::InvalidOffer);
    }
    let record = state
        .world
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let BusinessOwner::Organization(seller) = record.owner() else {
        return Err(BusinessAcquisitionError::OfferDeclined);
    };
    if seller == buyer
        || state.player_organization() == Some(seller)
        || state.economy.get_business_economy(business).is_none()
        || state
            .enterprises
            .enterprises_supported_by_business(business)
            .chain(
                state
                    .enterprises
                    .enterprises_at(crate::enterprises::EnterpriseLocation::Business(business)),
            )
            .any(|enterprise| enterprise.status() != crate::enterprises::EnterpriseStatus::Retired)
    {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    if validate_transfer_business_ownership(state, business, BusinessOwner::Organization(buyer))
        .is_err()
    {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    let economics = registry.get_business(record.kind()).economics();
    let expected_net =
        business_economy_system::forecast_normal_business_net(registry, state, business)?;
    // Use a common calendar horizon even for business definitions with different cycle lengths.
    let foregone_cents = i128::from(expected_net.cents().max(0)) * 90 * 1440
        / i128::from(economics.cycle().as_minutes());
    let minimum = i128::from(economics.acquisition_cost().cents()) + foregone_cents;
    if i128::from(offer.cents()) < minimum {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    Ok(seller)
}

/// Submit a binding offer for a non-player organization's property. Unlike a listing, the
/// price is chosen by the buyer and the owner can refuse. A successful offer uses the same
/// atomic conveyance, operating-book restart and lead dismissal as a direct purchase.
pub fn validate_offer_business(
    registry: &Registry,
    state: &AppState,
    draft: BusinessAcquisitionDraft,
    offer: Money,
) -> Result<ValidatedBusinessAcquisition, BusinessAcquisitionError> {
    if state.world.get_organization(draft.organization).is_none() {
        return Err(BusinessAcquisitionError::MissingOrganization(
            draft.organization,
        ));
    }
    validate_funding_accounts(state, draft.organization, &draft.funding_accounts, offer)?;
    let seller = eligible_offer_seller(registry, state, draft.organization, draft.business, offer)?;
    let business = state
        .world
        .get_business(draft.business)
        .expect("offer proved the business exists");
    Ok(ValidatedBusinessAcquisition {
        economy_cycle_duration: registry.get_business(business.kind()).economics().cycle(),
        funding_accounts: draft.funding_accounts,
        price: offer,
        business: draft.business,
        business_name: business.name().to_owned(),
        organization: draft.organization,
        negotiated_seller: Some((seller, business.version())),
    })
}

#[cfg(test)]
mod tests;
