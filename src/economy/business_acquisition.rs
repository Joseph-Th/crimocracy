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
use crate::core::time::{SimTime, ensure_time_current};
use crate::economy::business_economy_system;
use crate::finance::finance_system::FinanceError;
use crate::finance::helpers::format_money_cents;
use crate::finance::{FinancialOwner, Money};
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
use crate::world::{BusinessFunction, BusinessOwner, OrganizationKind, TraitKind};
use std::collections::BTreeSet;
use thiserror::Error;

mod autonomous;
mod payment;
mod portfolio_forecast;
mod seller_strategy;
pub(crate) use autonomous::apply_due_autonomous_acquisitions;
use payment::validate_funding_accounts;
use seller_strategy::{negotiated_reference_price, seller_preserves_earning_base};

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
        "the owner declined your acquisition offer; price or strategic commitments may prevent a sale"
    )]
    OfferDeclined,
    #[error("an acquisition offer must be positive")]
    InvalidOffer,
    #[error("the buyer-specific business acquisition quote exceeds supported account capacity")]
    PriceOverflow,
    #[error("acquisition validated at {expected:?} cannot commit at {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("the listing for business {0} changed after acquisition validation")]
    StaleListing(BusinessId),
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

/// An appraisal derived only from public directory and neighborhood economics. Neither
/// ownership, operating status, accounts nor a seller's willingness enters this estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusinessAppraisal {
    pub standard_price: Money,
    pub expected_cycle_net: Money,
    pub cycle_minutes: u32,
    pub reference_offer_cents: i128,
}

pub fn appraise_business(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<BusinessAppraisal, BusinessAcquisitionError> {
    let record = state
        .world()
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let economics = registry.get_business(record.kind()).economics();
    let standard_price = economics.acquisition_cost();
    let cycle_minutes = economics.cycle().as_minutes();
    let expected_cycle_net =
        business_economy_system::forecast_normal_business_net(registry, state, business)?;
    Ok(BusinessAppraisal {
        standard_price,
        expected_cycle_net,
        cycle_minutes,
        reference_offer_cents: seller_strategy::appraisal_reference_offer(
            standard_price,
            expected_cycle_net,
            cycle_minutes,
        ),
    })
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
    validated_at: SimTime,
    expected_business_version: u32,
    negotiated_seller: Option<FinancialOwner>,
}

impl ValidatedBusinessAcquisition {
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<AcquiredBusiness, BusinessAcquisitionError> {
        // Recheck the clock and listed ownership revision before planning any effects.
        // Same-minute changes in solvency, dependencies and leads still require live preflight.
        ensure_time_current(state.now(), self.validated_at)
            .map_err(|(expected, found)| BusinessAcquisitionError::StaleTime { expected, found })?;
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessAcquisitionError::MissingBusiness(self.business))?;
        if let Some(seller) = self.negotiated_seller {
            if business_record.version() != self.expected_business_version
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
        } else if business_record.version() != self.expected_business_version
            || listed_acquisition_price(registry, state, self.organization, self.business)?
                != self.price
        {
            return Err(BusinessAcquisitionError::StaleListing(self.business));
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

        let (payment, establishment) = self.plan_payment(state, existing_economy)?;
        let restart = if existing_economy.is_some() {
            Some(business_economy_system::validate_acquisition_restart(
                state,
                self.business,
                self.economy_cycle_duration,
                self.negotiated_seller.is_none(),
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

        let seller_notice = self.negotiated_seller.and_then(|seller| match seller {
            FinancialOwner::Organization(organization) => Some(organization),
            FinancialOwner::Character(_) | FinancialOwner::Business(_) => None,
        }).map(|seller| {
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

/// Growing a sprawling criminal holding company requires legal diligence,
/// financing and management integration beyond the third deed. The quoted
/// premium applies to independent listings and negotiated buyouts alike.
/// It affects the actual balanced purchase payment, not later shop income.
fn consolidation_percentage(state: &AppState, buyer: OrganizationId) -> i128 {
    consolidation_percentage_for_count(
        state
            .world()
            .businesses_owned_by_organization(buyer)
            .count(),
    )
}

fn consolidation_percentage_for_count(owned: usize) -> i128 {
    100 + i128::try_from(owned.saturating_sub(2).min(12)).expect("bounded holding count") * 50
}

/// The same consolidation premium applies to listings and voluntary offers. NPCs must
/// fund the real buyer-specific price, not repeatedly submit an impossible unadjusted bid.
fn buyer_adjusted_price(base_cents: i128, owned: usize) -> Result<Money, BusinessAcquisitionError> {
    let quoted = base_cents.saturating_mul(consolidation_percentage_for_count(owned)) / 100;
    Ok(Money::from_cents(
        i64::try_from(quoted).map_err(|_| BusinessAcquisitionError::PriceOverflow)?,
    ))
}

fn listed_acquisition_price(
    registry: &Registry,
    state: &AppState,
    buyer: OrganizationId,
    business: BusinessId,
) -> Result<Money, BusinessAcquisitionError> {
    let record = state
        .world()
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let base = registry
        .get_business(record.kind())
        .economics()
        .acquisition_cost();
    buyer_adjusted_price(
        i128::from(base.cents()),
        state
            .world()
            .businesses_owned_by_organization(buyer)
            .count(),
    )
}

/// Public quote for an independent listing, including the buyer's consolidation premium.
/// It reveals only public infrastructure and price, not rival books.
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
        listed_acquisition_price(registry, state, organization, business).ok()?,
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
    let price = listed_acquisition_price(registry, state, draft.organization, draft.business)?;
    validate_funding_accounts(state, draft.organization, &draft.funding_accounts, price)?;
    // Ownership conflicts (active enterprise hosts/supports) reject through the canonical
    // transfer path; unchanged ownership is impossible because an Independent owner never
    // equals an Organization owner. Commit checks the ownership revision and re-runs
    // transfer validation against live enterprise dependencies.
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
        validated_at: state.now(),
        expected_business_version: business_record.version(),
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
) -> Result<FinancialOwner, BusinessAcquisitionError> {
    if offer <= Money::ZERO {
        return Err(BusinessAcquisitionError::InvalidOffer);
    }
    let record = state
        .world
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let seller = match record.owner() {
        BusinessOwner::Organization(seller) => FinancialOwner::Organization(seller),
        BusinessOwner::Character(seller) => FinancialOwner::Character(seller),
        BusinessOwner::Independent => return Err(BusinessAcquisitionError::OfferDeclined),
    };
    if matches!(seller, FinancialOwner::Organization(seller_id)
        if seller_id == buyer || state.player_organization() == Some(seller_id))
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
    // A staffed rival cannot dispose of its capital without the same available
    // executive required to commission a purchase. Preserve private consent
    // and the liquidation path for holdings with no remaining personnel.
    if let FinancialOwner::Organization(seller_id) = seller
        && state
            .world()
            .get_organization(seller_id)
            .is_some_and(|organization| organization.kind() == OrganizationKind::Criminal)
        && state
            .world()
            .characters_in_organization(seller_id)
            .next()
            .is_some()
        && crate::world::world_system::sole_available_executive(state, seller_id).is_none()
    {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    if validate_transfer_business_ownership(state, business, BusinessOwner::Organization(buyer))
        .is_err()
    {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    let minimum = match seller {
        FinancialOwner::Organization(_) => negotiated_reference_price(registry, state, business)?,
        FinancialOwner::Character(person) => {
            personal_seller_minimum(registry, state, person, business)?
        }
        FinancialOwner::Business(_) => unreachable!("business is not a personal proprietor"),
    };
    if i128::from(offer.cents())
        < minimum.saturating_mul(consolidation_percentage(state, buyer)) / 100
        || matches!(seller, FinancialOwner::Organization(organization)
            if seller_preserves_earning_base(registry, state, organization, business, offer)?)
    {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    Ok(seller)
}

/// A registered person's choice to part with an actual livelihood. Reliable
/// profits, professional standing and disposition all influence the price;
/// exhaustion can make an owner more willing to exit. A single-shop proprietor
/// requires another 90 days of expected earnings to compensate for losing work.
/// This is a private reservation price, not an advertised market quotation.
fn personal_seller_minimum(
    registry: &Registry,
    state: &AppState,
    seller: crate::core::id::CharacterId,
    business: BusinessId,
) -> Result<i128, BusinessAcquisitionError> {
    let person = state
        .world()
        .get_character(seller)
        .ok_or(BusinessAcquisitionError::OfferDeclined)?;
    if state.legal().active_arrest_for_character(seller).is_some() {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    let base = negotiated_reference_price(registry, state, business)?;
    let only_livelihood = state.world().businesses_owned_by_character(seller).count() == 1;
    let extra_income = if only_livelihood {
        seller_strategy::daily_net(registry, state, business)?.max(0) * 90
    } else {
        0
    };
    let standing = i128::from(person.life().professional_standing());
    let fatigue = i128::from(person.life().fatigue());
    let stubbornness = if person.has_trait(TraitKind::Greedy) {
        18
    } else if person.has_trait(TraitKind::Proud) {
        12
    } else if person.has_trait(TraitKind::Cautious) {
        8
    } else if person.has_trait(TraitKind::Impulsive) {
        -8
    } else {
        0
    };
    let multiplier = (90 + standing / 4 - fatigue / 3 + stubbornness).clamp(65, 150);
    Ok(base.saturating_add(extra_income).saturating_mul(multiplier) / 100)
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
        validated_at: state.now(),
        expected_business_version: business.version(),
        negotiated_seller: Some(seller),
    })
}

#[cfg(test)]
mod tests;
