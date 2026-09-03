use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::elements::AssetId;
use simplex::simplicityhl::elements::hashes::{Hash as _, sha256};

use crate::programs::swaption_lending::PositionTerms;

/// The witness-side shape of the terms: exactly the covenant's `Terms`
/// tuple, in its order (u256 as 32 bytes, integers as themselves).
pub type WitnessTerms = ([u8; 32], [u8; 32], u64, u64, u32, [u8; 32], [u8; 32], [u8; 32], [u8; 32], [u8; 32], u32);

/// Everything a v4 position commits to, plus the network.
#[derive(Debug, Clone, Copy)]
pub struct PositionParametersV4 {
    pub collateral_asset_id: AssetId,
    pub cash_asset_id: AssetId,
    pub borrower_nft_asset_id: AssetId,
    pub lender_nft_asset_id: AssetId,
    /// SHA-256 of the scriptPubKey the exercise cash is paid to.
    pub lender_payout_script_hash: [u8; 32],
    /// SHA-256 of the borrower's payout scriptPubKey (where the venue pays
    /// the borrower on a last look; policy, not enforced).
    pub borrower_payout_script_hash: [u8; 32],
    /// SHA-256 of the venue's last-look scriptPubKey: input 1 of a last look
    /// must be a coin at that script.
    pub last_look_script_hash: [u8; 32],
    /// From this height the venue may exercise on the borrower's behalf.
    pub last_look_height: u32,
    pub terms: PositionTerms,
    pub network: SimplicityNetwork,
}

impl PositionParametersV4 {
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
            self.borrower_payout_script_hash,
            self.last_look_script_hash,
            self.last_look_height,
        )
    }

    /// The digest the covenant recomputes: SHA-256 over the terms in
    /// witness order, integers big-endian. Storage slot 0 holds it.
    pub fn digest(&self) -> [u8; 32] {
        let (a, b, c, d, e, f, g, h, i, j, k) = self.witness_terms();
        let mut m = Vec::with_capacity(32 * 7 + 8 + 8 + 4 + 4);
        m.extend_from_slice(&a);
        m.extend_from_slice(&b);
        m.extend_from_slice(&c.to_be_bytes());
        m.extend_from_slice(&d.to_be_bytes());
        m.extend_from_slice(&e.to_be_bytes());
        m.extend_from_slice(&f);
        m.extend_from_slice(&g);
        m.extend_from_slice(&h);
        m.extend_from_slice(&i);
        m.extend_from_slice(&j);
        m.extend_from_slice(&k.to_be_bytes());
        sha256::Hash::hash(&m).to_byte_array()
    }
}
