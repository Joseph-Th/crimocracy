//! Acquiring an independently owned business as an organizational asset.
//!
//! The canonical purchase path: an organization buys a business outright at its authored
//! kind price, paid in full from accounted funds. Dirty street cash cannot become a
//! storefront — legitimate expansion is gated on laundering throughput, so the money
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
use crate::finance::finance_system::{
    insert_account, remove_unused_account, validate_record_transaction, FinanceError,
};
use crate::finance::helpers::format_money_cents;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::validate_transfer_business_ownership;
use crate::world::world_system::WorldError;
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
    /// Authored cycle duration for the economy commit opens; captured at validation time.
    economy_cycle_duration: crate::core::time::SimDuration,
    funding_account: FinancialAccountId,
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
        // ---- Phase 1: re-validate every fallible condition against live state. --------
        // A validated token can be held across intervening mutations, so solvency,
        // ownership, and hosting conflicts are re-checked before anything mutates.
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessAcquisitionError::MissingBusiness(self.business))?;
        if business_record.owner() != BusinessOwner::Independent {
            return Err(BusinessAcquisitionError::NotIndependentlyOwned {
                business: self.business,
                owner: business_record.owner(),
            });
        }
        let funding = state.finance.get_account(self.funding_account).ok_or(
            BusinessAcquisitionError::MissingFundingAccount(self.funding_account),
        )?;
        if funding.owner() != FinancialOwner::Organization(self.organization) {
            return Err(BusinessAcquisitionError::FundingAccountOwnerMismatch {
                account: self.funding_account,
                organization: self.organization,
            });
        }
        if funding.kind() != AccountKind::AccountedFunds {
            return Err(BusinessAcquisitionError::InvalidFundingAccountKind(
                self.funding_account,
            ));
        }
        if funding.balance() < self.price {
            return Err(BusinessAcquisitionError::InsufficientFunds {
                balance_cents: funding.balance().cents(),
                price_cents: self.price.cents(),
            });
        }
        // Hosting conflicts (active enterprise venues/supporters) reject through the same
        // canonical read the ownership transfer enforces.
        validate_transfer_business_ownership(
            state,
            self.business,
            BusinessOwner::Organization(self.organization),
        )?;
        let existing_economy = state.economy.get_business_economy(self.business);
        if existing_economy.is_some_and(|economy| {
            economy.status() != crate::economy::BusinessOperatingStatus::Active
        }) {
            return Err(
                business_economy_system::BusinessEconomyError::EconomyNotActive(self.business)
                    .into(),
            );
        }
        // The establishment decision follows live state, not the validation-time snapshot:
        // whether the target has ever operated is re-derived here so a token held across
        // someone else's establishment adopts the existing books instead of colliding.
        let establish_now = existing_economy.is_none();

        // ---- Phase 2: reserve fresh books when the target has never operated. --------
        // Insertion is purely additive; if any later leg rejects, the untouched accounts
        // are removed again so rejection leaves authoritative state unchanged.
        let mut reserved_accounts: Vec<FinancialAccountId> = Vec::new();
        let operating_account = if let Some(economy) = existing_economy {
            economy.operating_account()
        } else {
            // Nothing authoritative has mutated before the first reservation, so it needs
            // no rollback; only a failure while reserving the second account does.
            let operating = insert_account(
                state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Business(self.business),
                    kind: AccountKind::LegitimateOperating,
                },
            )?;
            reserved_accounts.push(operating);
            match insert_account(
                state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Business(self.business),
                    kind: AccountKind::Settlement,
                },
            ) {
                Ok(settlement) => reserved_accounts.push(settlement),
                Err(error) => {
                    for account in reserved_accounts.drain(..) {
                        remove_unused_account(state, account);
                    }
                    return Err(error.into());
                }
            }
            operating
        };

        // ---- Phase 3: pre-validate the payment leg and the report leg. ---------------
        // Both validators are read-only, so a rejection here rolls back only the reserved
        // accounts above and leaves every authoritative record untouched.
        let payment = match validate_record_transaction(
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
        ) {
            Ok(transaction) => transaction,
            Err(error) => {
                for account in reserved_accounts.drain(..) {
                    remove_unused_account(state, account);
                }
                return Err(error.into());
            }
        };
        let announcement = match validate_record_report(
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
        ) {
            Ok(report) => report,
            Err(error) => {
                for account in reserved_accounts.drain(..) {
                    remove_unused_account(state, account);
                }
                return Err(error.into());
            }
        };

        // ---- Phase 4: commit the validated legs. --------------------------------------
        // Every residual failure mode was excluded in phases 1-3: the transfer's conflict
        // and staleness re-checks passed above with exclusive state access since, the fresh
        // accounts satisfy establishment validation by construction, the payment's version
        // pins cover accounts no interim step touched, and the report references live
        // entities only.
        let transfer = validate_transfer_business_ownership(
            state,
            self.business,
            BusinessOwner::Organization(self.organization),
        )
        .expect("acquisition transfer re-validated immediately above must still validate");
        transfer
            .commit(state)
            .expect("a just-revalidated ownership transfer must commit atomically");
        if establish_now {
            let settlement_account = reserved_accounts[1];
            business_economy_system::ValidatedBusinessEconomyEstablishment::over_accounts_to_be_reserved(
                BusinessEconomyDraft {
                    business: self.business,
                    operating_account,
                    settlement_account,
                },
                self.economy_cycle_duration,
            )
            .commit(state)
            .expect("establishment over freshly reserved, construction-guaranteed accounts must commit");
        }
        payment
            .commit(state)
            .expect("a payment pre-validated against untouched account versions must commit");
        announcement
            .commit(state)
            .expect("a validated acquisition report about live entities must commit");
        Ok(AcquiredBusiness {
            business: self.business,
            price: self.price,
            established_economy: establish_now,
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
    // equals an Organization owner. Commit re-runs this read against live state, so the
    // token itself carries no transfer snapshot.
    validate_transfer_business_ownership(
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
        economy_cycle_duration: registry
            .get_business(business_record.kind())
            .economics()
            .cycle(),
        funding_account: draft.funding_account,
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

    /// A validated token held across other spending must not overdraw accounted funds:
    /// commit re-checks solvency and rejects without touching ownership or books.
    #[test]
    fn stale_token_rejects_when_funding_drains_before_commit() {
        let mut fixture = make_independent_fixture();
        let price = hospitality_price(&fixture);
        fund_accounted_from_street(&mut fixture, price.cents());

        let token = validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
        )
        .expect("a funded acquisition must validate");

        // Another validated spend drains the funding account after validation.
        let drain = validate_record_transaction(
            &fixture.state,
            LedgerTransactionDraft {
                occurred_at: fixture.state.now(),
                memo: "Interleaved spend".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: fixture.accounted,
                        amount: Money::from_cents(-price.cents()),
                    },
                    LedgerPosting {
                        account: fixture.street,
                        amount: Money::from_cents(price.cents()),
                    },
                ],
                authorization: None,
            },
        )
        .expect("drain transfer should validate")
        .commit(&mut fixture.state)
        .expect("drain transfer should commit");
        let _ = drain;

        let error = token
            .commit(&mut fixture.state)
            .expect_err("a stale token whose funding drained must reject at commit");
        assert_eq!(
            error,
            BusinessAcquisitionError::InsufficientFunds {
                balance_cents: 0,
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
            "the rejected stale purchase leaves ownership untouched"
        );
        assert!(
            fixture
                .state
                .economy()
                .get_business_economy(fixture.business)
                .is_none(),
            "the rejected stale purchase opens no economy"
        );
        assert!(
            fixture
                .state
                .reports()
                .reports_for(fixture.organization)
                .find(|report| report.title() == "Business acquisition")
                .is_none(),
            "the rejected stale purchase publishes no report"
        );
        // The interleaved transfer restored both accounts to zero; nothing was minted or
        // lost anywhere in the failed purchase.
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.street)
                .expect("street account must persist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.accounted)
                .expect("accounted account must persist")
                .balance(),
            Money::ZERO
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
