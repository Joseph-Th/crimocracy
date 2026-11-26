//! Private owner income projections for commercial investment and sale consent.
//!
//! Candidate appraisals remain public estimates. The owner's existing shops use their
//! actual live market and disruption facts. The same portfolio administration model
//! and per-cycle fee rounding used by settlement price every projected receipt.

use super::{BusinessAcquisitionError, BusinessAppraisal};
use crate::core::id::{BusinessId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::economy::business_economy_system::{
    PortfolioAdministration, acquisition_owner_daily_displacement, daily_owner_net,
    forecast_acquisition_business_net, forecast_live_business_net,
};
use crate::finance::Money;
use crate::registry::Registry;

pub(super) struct PortfolioForecast {
    owner: OrganizationId,
    excluded: Option<BusinessId>,
    pub(super) administration: PortfolioAdministration,
    current: DailyProjection,
    local_purchase: DailyProjection,
    expansion_purchase: DailyProjection,
}

struct DailyProjection {
    overhead: u16,
    net: i128,
}

impl DailyProjection {
    fn new(overhead: u16) -> Self {
        Self { overhead, net: 0 }
    }

    fn include(&mut self, net: Money, cycle: u32) {
        self.net += daily_owner_net(net, cycle, self.overhead);
    }
}

impl PortfolioForecast {
    pub(super) fn new(
        registry: &Registry,
        state: &AppState,
        owner: OrganizationId,
        excluded: Option<BusinessId>,
    ) -> Result<Self, BusinessAcquisitionError> {
        let administration = PortfolioAdministration::for_owner(state, owner, excluded);
        let mut current = DailyProjection::new(administration.basis_points());
        let mut local_purchase =
            DailyProjection::new(administration.basis_points_after_purchase(false));
        let mut expansion_purchase =
            DailyProjection::new(administration.basis_points_after_purchase(true));
        for asset in state.world().businesses_owned_by_organization(owner) {
            if Some(asset.id()) == excluded
                || state
                    .economy()
                    .get_business_economy(asset.id())
                    .is_none_or(|economy| !economy.is_operating())
            {
                continue;
            }
            let cycle = registry
                .get_business(asset.kind())
                .economics()
                .cycle()
                .as_minutes();
            let net = forecast_live_business_net(registry, state, asset.id())?;
            current.include(net, cycle);
            local_purchase.include(net, cycle);
            expansion_purchase.include(net, cycle);
        }
        Ok(Self {
            owner,
            excluded,
            administration,
            current,
            local_purchase,
            expansion_purchase,
        })
    }

    pub(super) fn daily_net(&self) -> i128 {
        self.current.net
    }

    pub(super) fn purchase_gain(
        &self,
        registry: &Registry,
        state: &AppState,
        candidate: BusinessId,
        district: NeighborhoodId,
        appraisal: &BusinessAppraisal,
    ) -> Result<i128, BusinessAcquisitionError> {
        let projection = if self.administration.contains_district(district) {
            &self.local_purchase
        } else {
            &self.expansion_purchase
        };
        let overhead = projection.overhead;
        let incoming = daily_owner_net(
            forecast_acquisition_business_net(registry, state, candidate, self.owner)?,
            appraisal.cycle_minutes,
            overhead,
        );
        let displaced = acquisition_owner_daily_displacement(
            registry,
            state,
            candidate,
            self.owner,
            overhead,
            self.excluded,
        )?;
        Ok(projection.net + incoming - displaced - self.daily_net())
    }
}
