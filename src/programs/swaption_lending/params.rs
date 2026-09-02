use simplex::{provider::SimplicityNetwork, simplicityhl::elements::AssetId};

use crate::{
    artifacts::swaption_lending::derived_swaption_lending::SwaptionLendingArguments,
    programs::swaption_lending::PositionTerms,
};

/// Everything the covenant's script hash commits to, plus the network.
#[derive(Debug, Clone, Copy)]
pub struct SwaptionPositionParameters {
    pub collateral_asset_id: AssetId,
    pub cash_asset_id: AssetId,
    pub borrower_nft_asset_id: AssetId,
    pub lender_nft_asset_id: AssetId,
    /// Script hash (SHA-256 of the scriptPubKey) the exercise cash is paid to.
    pub lender_payout_script_hash: [u8; 32],
    pub terms: PositionTerms,
    pub network: SimplicityNetwork,
}

impl SwaptionPositionParameters {
    pub fn build_arguments(&self) -> SwaptionLendingArguments {
        SwaptionLendingArguments {
            collateral_asset_id: self.collateral_asset_id.into_inner().0,
            cash_asset_id: self.cash_asset_id.into_inner().0,
            collateral_amount: self.terms.collateral_amount,
            buyback_amount: self.terms.buyback_amount,
            expiry_height: self.terms.expiry_height,
            borrower_nft_asset_id: self.borrower_nft_asset_id.into_inner().0,
            lender_nft_asset_id: self.lender_nft_asset_id.into_inner().0,
            lender_payout_script_hash: self.lender_payout_script_hash,
        }
    }
}
