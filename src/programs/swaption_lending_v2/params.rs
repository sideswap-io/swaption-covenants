use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::elements::AssetId;
use simplex::simplicityhl::elements::hashes::{Hash as _, sha256};

use crate::programs::swaption_lending::PositionTerms;

/// The witness-side shape of the terms: exactly the covenant's `Terms`
/// tuple, in its order (u256 as 32 bytes, integers as themselves).
pub type WitnessTerms = ([u8; 32], [u8; 32], u64, u64, u32, [u8; 32], [u8; 32], [u8; 32]);

/// Everything a v2 position commits to, plus the network.
#[derive(Debug, Clone, Copy)]
pub struct PositionParametersV2 {
    pub collateral_asset_id: AssetId,
    pub cash_asset_id: AssetId,
    pub borrower_nft_asset_id: AssetId,
    pub lender_nft_asset_id: AssetId,
    /// SHA-256 of the scriptPubKey the exercise cash is paid to.
    pub lender_payout_script_hash: [u8; 32],
    pub terms: PositionTerms,
    pub network: SimplicityNetwork,
}

impl PositionParametersV2 {
    pub fn witness_terms(&self) -> WitnessTerms {
        (
            self.collateral_asset_id.into_inner().0,
            self.cash_asset_id.into_inner().0,
            self.terms.collateral_amount,
            self.terms.buyback_amount,
            self.terms.expiry_height,
            self.borrower_nft_asset_id.into_inner().0,
            self.lender_nft_asset_id.into_inner().0,
            self.lender_payout_script_hash,
        )
    }

    /// The digest the covenant recomputes: SHA-256 over the terms in
    /// witness order, integers big-endian. Storage slot 0 holds it.
    pub fn digest(&self) -> [u8; 32] {
        let (a, b, c, d, e, f, g, h) = self.witness_terms();
        let mut m = Vec::with_capacity(32 * 5 + 8 + 8 + 4);
        m.extend_from_slice(&a);
        m.extend_from_slice(&b);
        m.extend_from_slice(&c.to_be_bytes());
        m.extend_from_slice(&d.to_be_bytes());
        m.extend_from_slice(&e.to_be_bytes());
        m.extend_from_slice(&f);
        m.extend_from_slice(&g);
        m.extend_from_slice(&h);
        sha256::Hash::hash(&m).to_byte_array()
    }
}
