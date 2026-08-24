//! Acquiring an independently owned business as an organizational asset.
//!
//! The canonical purchase path: an organization buys a business outright at its authored
//! kind price, paid in full from accounted funds. Dirty street cash cannot become a
//! storefront â€” legitimate expansion is gated on laundering throughput, so the money
//! loop closes: illicit proceeds are laundered into accounted wealth, and accounted
//! wealth converts into earning capacity (and, for cash-intensive fronts, more
//! laundering capacity). Independently owned businesses have no organizational
//! counterparty to pay, so the full price capitalizes the acquired business's own
//! operating books.
//!
//! Commit composes three canonical paths in one validated step: world ownership
//! transfer, business-economy establishment when the target has never operated, and a
//! balanced ledger payment. Every fallible condition is validated before any mutation;
//! the records constructed during commit reference freshly reserved accounts whose kinds
//! and owners are guaranteed by construction.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessId, FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::economy::{business_economy_system, BusinessEconomyDraft};
use crate::finance::finance_system::{insert_account, validate_record_transaction, FinanceError};
use crate::finance::helpers::format_money_cents;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::{
    validate_transfer_business_ownership, ValidatedBusinessOwnershipTransfer, WorldError,
};
use crate::world::BusinessOwner;
use thiserror::Error;

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
        "accounted funds {balance_cents} cannot cover the {price_cents}-cent acquisition price"
    )]
    InsufficientFunds {
        balance_cents: i64,
        price_cents: i64,
    },
    #[error(transparent)]
    World(#[from] WorldError),
    #[error(transparent)]
    Economy(#[from] business_economy_system::BusinessEconomyError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Report(#[from] ReportError),
}

#[derive(Clone, Debug)]
pub struct BusinessAcquisitionDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    /// Must be one of the organization's accounted-funds accounts.
    pub funding_account: FinancialAccountId,
}

/// What one committed acquisition actually did, quoted from production state rather than
/// recomputed by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquiredBusiness {
    pub business: BusinessId,
    pub price: Money,
    /// True when commit also opened the acquired business's first operating economy.
    pub established_economy: bool,
}

#[derive(Debug)]
pub struct ValidatedBusinessAcquisition {
    transfer: ValidatedBusinessOwnershipTransfer,
    /// Set when the target has never operated and commit must open its first economy over
    /// freshly reserved accounts.
    establish_economy: bool,
    /// Authored cycle duration for the economy commit opens; captured at validation time.
    economy_cycle_duration: crate::core::time::SimDuration,
    funding_account: FinancialAccountId,
    operating_account: Option<FinancialAccountId>,
    price: Money,
    business: BusinessId,
    business_name: String,
    organization: OrganizationId,
}

impl ValidatedBusinessAcquisition {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<AcquiredBusiness, BusinessAcquisitionError> {
        // Ownership moves first: every later record describes property the organization
        // already holds. The transfer was validated against the owner snapshot up front.
        self.transfer.commit(state)?;
        let operating_account = if self.establish_economy {
            let operating = insert_account(
                state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Business(self.business),
                    kind: AccountKind::LegitimateOperating,
                },
            )
            .expect("a fresh operating account for an existing business must validate");
            let settlement = insert_account(
                state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Business(self.business),
                    kind: AccountKind::Settlement,
                },
            )
            .expect("a fresh settlement account for an existing business must validate");
            business_economy_system::ValidatedBusinessEconomyEstablishment::over_accounts_to_be_reserved(
                BusinessEconomyDraft {
                    business: self.business,
                    operating_account: operating,
                    settlement_account: settlement,
                },
                self.economy_cycle_duration,
            )
            .commit(state)?;
            operating
        } else {
            self.operating_account
                .expect("an operating target always carries its economy's operating account")
        };
        validate_record_transaction(
            state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: format!("Business acquisition of {}", self.business_name),
                postings: vec![
                    LedgerPosting {
                        account: self.funding_account,
                        amount: self
                            .price
                            .checked_neg()
                            .expect("an authored acquisition price must fit negation"),
                    },
                    LedgerPosting {
                        account: operating_account,
                        amount: self.price,
                    },
                ],
                authorization: None,
            },
        )?
        .commit(state)?;
        // The organization legitimately knows what it bought and what it paid: surface the
        // acquisition through the canonical report path so the next executive brief carries it.
        validate_record_report(
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
        )?
        .commit(state)?;
        Ok(AcquiredBusiness {
            business: self.business,
            price: self.price,
            established_economy: self.establish_economy,
        })
    }
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
    let funding = state.finance.get_account(draft.funding_account).ok_or(
        BusinessAcquisitionError::MissingFundingAccount(draft.funding_account),
    )?;
    if funding.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(BusinessAcquisitionError::FundingAccountOwnerMismatch {
            account: draft.funding_account,
            organization: draft.organization,
        });
    }
    if funding.kind() != AccountKind::AccountedFunds {
        return Err(BusinessAcquisitionError::InvalidFundingAccountKind(
            draft.funding_account,
        ));
    }
    if funding.balance() < price {
        return Err(BusinessAcquisitionError::InsufficientFunds {
            balance_cents: funding.balance().cents(),
            price_cents: price.cents(),
        });
    }
    // Ownership conflicts (active enterprise hosts/supports) reject through the canonical
    // transfer path; unchanged ownership is impossible because an Independent owner never
    // equals an Organization owner.
    let transfer = validate_transfer_business_ownership(
        state,
        draft.business,
        BusinessOwner::Organization(draft.organization),
    )?;
    let existing_economy = state.economy.get_business_economy(draft.business);
    if existing_economy
        .is_some_and(|economy| economy.status() != crate::economy::BusinessOperatingStatus::Active)
    {
        return Err(
            business_economy_system::BusinessEconomyError::EconomyNotActive(draft.business).into(),
        );
    }
    Ok(ValidatedBusinessAcquisition {
        transfer,
        establish_economy: existing_economy.is_none(),
        economy_cycle_duration: registry
            .get_business(business_record.kind())
            .economics()
            .cycle(),
        funding_account: draft.funding_account,
        operating_account: existing_economy.map(|economy| economy.operating_account()),
        price,
        business: draft.business,
        business_name: business_record.name().to_owned(),
        organization: draft.organization,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::finance::finance_system::insert_account;
    use crate::world::world_system::{insert_business, insert_neighborhood, insert_organization};
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, NeighborhoodDraft,
        NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile,
        OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::BTreeSet;

    const SEED: u64 = 0x0AC0_5171;

    struct AcquisitionFixture {
        registry: Registry,
        state: AppState,
        organization: OrganizationId,
        business: BusinessId,
        accounted: FinancialAccountId,
        street: FinancialAccountId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn make_independent_fixture() -> AcquisitionFixture {
        let registry = build_registry();
        let mut state = AppState::new(SEED);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Marlowe Holdings".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Harbor Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(55),
                        commercial_activity: rating(60),
                        illicit_demand: rating(40),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(30),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Pier Nine Social Club".to_owned(),
                kind: BusinessKind::Hospitality,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::MeetingSpace,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("business fixture should validate");
        let accounted = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::AccountedFunds,
            },
        )
        .expect("accounted-funds fixture should validate");
        let street = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
            },
        )
        .expect("street-cash fixture should validate");
        AcquisitionFixture {
            registry,
            state,
            organization,
            business,
            accounted,
            street,
        }
    }

    fn hospitality_price(fixture: &AcquisitionFixture) -> Money {
        fixture
            .registry
            .get_business(BusinessKind::Hospitality)
            .economics()
            .acquisition_cost()
    }

    fn fund_accounted_from_street(fixture: &mut AcquisitionFixture, cents: i64) {
        validate_record_transaction(
            &fixture.state,
            LedgerTransactionDraft {
                occurred_at: fixture.state.now(),
                memo: "Fixture capitalization".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: fixture.street,
                        amount: Money::from_cents(-cents),
                    },
                    LedgerPosting {
                        account: fixture.accounted,
                        amount: Money::from_cents(cents),
                    },
                ],
                authorization: None,
            },
        )
        .expect("fixture capitalization should validate")
        .commit(&mut fixture.state)
        .expect("fixture capitalization should commit");
    }

    fn acquisition_draft(fixture: &AcquisitionFixture) -> BusinessAcquisitionDraft {
        BusinessAcquisitionDraft {
            organization: fixture.organization,
            business: fixture.business,
            funding_account: fixture.accounted,
        }
    }

    #[test]
    fn acquisition_buys_an_independent_business_and_capitalizes_its_books() {
        let mut fixture = make_independent_fixture();
        let price = hospitality_price(&fixture);
        fund_accounted_from_street(&mut fixture, price.cents());

        let acquired = validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
        )
        .expect("a funded independent acquisition must validate")
        .commit(&mut fixture.state)
        .expect("a validated acquisition must commit");

        assert_eq!(acquired.business, fixture.business);
        assert_eq!(acquired.price, price);
        assert!(acquired.established_economy);

        // Ownership moved through the canonical record.
        let business = fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("acquired business must persist");
        assert_eq!(
            business.owner(),
            BusinessOwner::Organization(fixture.organization)
        );

        // The full price capitalized the fresh operating books; accounted funds dropped by
        // exactly the authored price.
        let economy = fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("an acquisition of an unoperated business must open its economy");
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(economy.operating_account())
                .expect("operating account must persist")
                .balance(),
            price
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.accounted)
                .expect("funding account must persist")
                .balance(),
            Money::ZERO
        );

        // The purchase surfaces as player-visible financial information.
        let report = fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .find(|report| report.kind() == ReportKind::Financial)
            .expect("the acquisition must surface as a financial report");
        assert_eq!(report.entries()[0].attention, AttentionClass::Notable);
        assert!(report.entries()[0]
            .summary
            .contains("Pier Nine Social Club"));

        validate_invariants(&fixture.state);
    }

    #[test]
    fn short_accounted_funds_reject_the_purchase_without_touching_state() {
        let mut fixture = make_independent_fixture();
        let price = hospitality_price(&fixture);
        fund_accounted_from_street(&mut fixture, price.cents() - 1);

        let error = validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
        )
        .expect_err("a short purchase must reject");

        assert_eq!(
            error,
            BusinessAcquisitionError::InsufficientFunds {
                balance_cents: price.cents() - 1,
                price_cents: price.cents(),
            }
        );
        assert_eq!(
            fixture
                .state
                .world()
                .get_business(fixture.business)
                .expect("business must persist")
                .owner(),
            BusinessOwner::Independent,
            "a rejected acquisition leaves ownership untouched"
        );
        assert!(
            fixture
                .state
                .economy()
                .get_business_economy(fixture.business)
                .is_none(),
            "a rejected acquisition opens no economy"
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn dirty_street_cash_cannot_buy_a_legitimate_business() {
        let fixture = make_independent_fixture();

        let error = validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            BusinessAcquisitionDraft {
                funding_account: fixture.street,
                ..acquisition_draft(&fixture)
            },
        )
        .expect_err("dirty money must not buy legitimacy");

        assert_eq!(
            error,
            BusinessAcquisitionError::InvalidFundingAccountKind(fixture.street)
        );
        assert!(
            fixture
                .state
                .world()
                .get_business(fixture.business)
                .expect("business must persist")
                .owner()
                == BusinessOwner::Independent,
            "a rejected acquisition leaves ownership untouched"
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn foreign_owned_targets_are_not_purchasable() {
        let mut fixture = make_independent_fixture();
        let rival = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Rosetti Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("rival fixture should validate");
        crate::world::world_system::validate_transfer_business_ownership(
            &fixture.state,
            fixture.business,
            BusinessOwner::Organization(rival),
        )
        .expect("rival ownership fixture should validate")
        .commit(&mut fixture.state)
        .expect("rival ownership fixture should commit");

        let error = validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
        )
        .expect_err("a rival-owned venue is not on the market");

        assert_eq!(
            error,
            BusinessAcquisitionError::NotIndependentlyOwned {
                business: fixture.business,
                owner: BusinessOwner::Organization(rival),
            }
        );
        validate_invariants(&fixture.state);
    }
}
