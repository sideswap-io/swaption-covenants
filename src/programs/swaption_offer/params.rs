use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::elements::AssetId;
use simplex::simplicityhl::elements::hashes::{Hash as _, sha256};

use crate::programs::swaption_lending::PositionTerms;
use crate::programs::swaption_lending_v5::PositionParametersV5;

/// Quote rows per offer, fixed by the covenant's `Rows` type.
pub const OFFER_ROWS: usize = 4;

/// Prices are cash sats per WHOLE unit (1e8 sats) of collateral.
pub const SCALE: u64 = 100_000_000;

/// One row of the covenant's `Rows` tuple, in its order.
pub type WitnessRow = ([u8; 32], u32, u64, u64, u64, u64);
pub type WitnessRows = (WitnessRow, WitnessRow, WitnessRow, WitnessRow);
/// The covenant's `OfferTerms` tuple, in its order.
pub type WitnessOfferTerms = ([u8; 32], [u8; 32], [u8; 32], [u8; 32], [u8; 32], u64, u32, WitnessRows);

/// One quote of an offer: what the escrowed cash buys, at what price, until
/// when. An all-zero row is unused and cannot be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfferRow {
    pub collateral_asset_id: AssetId,
    pub expiry_height: u32,
    /// Cash sats released per whole unit of collateral (the sale price plus
    /// the lender's venue fee per unit).
    pub price_out: u64,
    /// Cash sats per whole unit the borrower must owe, at least.
    pub buyback_price: u64,
    /// Venue fee per whole unit of collateral (both sides).
    pub fee_per_unit: u64,
    /// Smallest fill, collateral sats.
    pub min_size: u64,
}

impl OfferRow {
    pub fn empty() -> Self {
        Self {
            collateral_asset_id: AssetId::from_slice(&[0u8; 32]).expect("32 bytes"),
            expiry_height: 0,
            price_out: 0,
            buyback_price: 0,
            fee_per_unit: 0,
            min_size: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.expiry_height == 0
    }

    pub fn witness(&self) -> WitnessRow {
        (
            self.collateral_asset_id.into_inner().0,
            self.expiry_height,
            self.price_out,
            self.buyback_price,
            self.fee_per_unit,
            self.min_size,
        )
    }

    /// Cash released for `n` collateral sats (floor). Mirrors `out` in the covenant.
    pub fn cash_out(&self, n: u64) -> u64 {
        mul_div(n, self.price_out)
    }

    /// Cash one minimum fill takes. Mirrors `min_cash`.
    pub fn min_cash(&self) -> u64 {
        mul_div(self.min_size, self.price_out)
    }

    /// The least the borrower must owe for `n`. Mirrors `buyback_floor`.
    pub fn buyback_floor(&self, n: u64) -> u64 {
        mul_div(n, self.buyback_price)
    }
}

fn mul_div(x: u64, price: u64) -> u64 {
    (u128::from(x) * u128::from(price) / u128::from(SCALE)) as u64
}

/// What a fill of `n` does to an offer holding `remaining`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillOutcome {
    /// Cash released from the escrow.
    pub out: u64,
    /// Cash left after the fill.
    pub rest: u64,
    /// `rest` is at least one minimum fill: the offer continues at output 2.
    pub continues: bool,
    /// The venue fee the covenant demands.
    pub fee_due: u64,
}

impl FillOutcome {
    /// `0 < rest < min_cash`: the leftover goes to the claim script at output 2.
    pub fn leftover(&self) -> bool {
        !self.continues && self.rest > 0
    }

    /// Index of the fee output: 3 after a continuation or leftover, 2 when
    /// the fill takes the last sat.
    pub fn fee_output_index(&self) -> usize {
        if self.rest > 0 { 3 } else { 2 }
    }
}

/// Everything an offer commits to, plus the network.
#[derive(Debug, Clone, Copy)]
pub struct OfferParameters {
    pub cash_asset_id: AssetId,
    /// The wallet's lender token: cancel authority, and the `lender_nft`
    /// term of every position the offer creates.
    pub lender_token_asset_id: AssetId,
    /// SHA-256 of the claim scriptPubKey bound to the lender token.
    pub claim_script_hash: [u8; 32],
    /// SHA-256 of the venue fee scriptPubKey.
    pub fee_script_hash: [u8; 32],
    /// Tapleaf hash of the position program the offer creates (v5).
    pub position_leaf: [u8; 32],
    /// Minimum venue fee per fill, cash sats.
    pub fee_min: u64,
    /// From this height anyone may return the cash to the claim script.
    pub cutoff_height: u32,
    pub rows: [OfferRow; OFFER_ROWS],
    pub network: SimplicityNetwork,
}

impl OfferParameters {
    pub fn witness_terms(&self) -> WitnessOfferTerms {
        (
            self.cash_asset_id.into_inner().0,
            self.lender_token_asset_id.into_inner().0,
            self.claim_script_hash,
            self.fee_script_hash,
            self.position_leaf,
            self.fee_min,
            self.cutoff_height,
            (self.rows[0].witness(), self.rows[1].witness(), self.rows[2].witness(), self.rows[3].witness()),
        )
    }

    /// The digest the covenant recomputes: SHA-256 over the terms in witness
    /// order, integers big-endian. Storage slot 0 holds it.
    pub fn digest(&self) -> [u8; 32] {
        let mut m = Vec::with_capacity(32 * 5 + 8 + 4 + OFFER_ROWS * (32 + 4 + 8 * 4));
        m.extend_from_slice(&self.cash_asset_id.into_inner().0);
        m.extend_from_slice(&self.lender_token_asset_id.into_inner().0);
        m.extend_from_slice(&self.claim_script_hash);
        m.extend_from_slice(&self.fee_script_hash);
        m.extend_from_slice(&self.position_leaf);
        m.extend_from_slice(&self.fee_min.to_be_bytes());
        m.extend_from_slice(&self.cutoff_height.to_be_bytes());
        for row in &self.rows {
            m.extend_from_slice(&row.collateral_asset_id.into_inner().0);
            m.extend_from_slice(&row.expiry_height.to_be_bytes());
            m.extend_from_slice(&row.price_out.to_be_bytes());
            m.extend_from_slice(&row.buyback_price.to_be_bytes());
            m.extend_from_slice(&row.fee_per_unit.to_be_bytes());
            m.extend_from_slice(&row.min_size.to_be_bytes());
        }
        sha256::Hash::hash(&m).to_byte_array()
    }

    pub fn row(&self, index: u8) -> &OfferRow {
        &self.rows[usize::from(index)]
    }

    /// The venue fee a fill of `n` on `row` must pay: the per-unit fee or the floor.
    pub fn fee_due(&self, row: u8, n: u64) -> u64 {
        mul_div(n, self.row(row).fee_per_unit).max(self.fee_min)
    }

    /// What a fill of `n` on `row` does to an offer holding `remaining`.
    /// Panics where the covenant would refuse.
    pub fn fill_outcome(&self, row: u8, remaining: u64, n: u64) -> FillOutcome {
        let r = self.row(row);
        assert!(!r.is_empty(), "empty row");
        assert!(n >= r.min_size, "below the minimum fill");
        let out = r.cash_out(n);
        assert!(out <= remaining, "fill exceeds the remaining cash");
        let rest = remaining - out;
        FillOutcome {
            out,
            rest,
            continues: rest >= r.min_cash(),
            fee_due: self.fee_due(row, n),
        }
    }

    /// The v5 position a fill of `n` on `row` creates. The lender side is
    /// fixed by the offer (lender token, claim script); the borrower side and
    /// the venue's last look are the filler's to choose.
    #[allow(clippy::too_many_arguments)]
    pub fn position_parameters(
        &self,
        row: u8,
        n: u64,
        buyback_amount: u64,
        borrower_nft_asset_id: AssetId,
        borrower_payout_script_hash: [u8; 32],
        last_look_script_hash: [u8; 32],
        last_look_height: u32,
    ) -> PositionParametersV5 {
        let r = self.row(row);
        PositionParametersV5 {
            collateral_asset_id: r.collateral_asset_id,
            cash_asset_id: self.cash_asset_id,
            borrower_nft_asset_id,
            lender_nft_asset_id: self.lender_token_asset_id,
            lender_payout_script_hash: self.claim_script_hash,
            borrower_payout_script_hash,
            last_look_script_hash,
            last_look_height,
            terms: PositionTerms {
                collateral_amount: n,
                buyback_amount,
                expiry_height: r.expiry_height,
            },
            network: self.network,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> OfferParameters {
        let asset = |b: u8| AssetId::from_slice(&[b; 32]).unwrap();
        let mut rows = [OfferRow::empty(); OFFER_ROWS];
        rows[0] = OfferRow {
            collateral_asset_id: asset(1),
            expiry_height: 3_200_000,
            price_out: 300_000_000,
            buyback_price: 400_000_000,
            fee_per_unit: 2_000_000,
            min_size: 1_000,
        };
        OfferParameters {
            cash_asset_id: asset(2),
            lender_token_asset_id: asset(4),
            claim_script_hash: [9u8; 32],
            fee_script_hash: [12u8; 32],
            position_leaf: [13u8; 32],
            fee_min: 50,
            cutoff_height: 3_199_000,
            rows,
            network: SimplicityNetwork::LiquidTestnet,
        }
    }

    #[test]
    fn fill_outcomes_follow_the_covenant() {
        let p = params();
        // 3000 sats at 3 cash per unit: 9000 out of 25,000, 16,000 left, continues.
        let o = p.fill_outcome(0, 25_000, 3_000);
        assert_eq!((o.out, o.rest, o.continues, o.fee_due), (9_000, 16_000, true, 60));
        assert_eq!(o.fee_output_index(), 3);
        // The floor binds below 2,500 sats.
        assert_eq!(p.fee_due(0, 1_000), 50);
        // 1000 sats out of 4000: 1000 left, below one minimum fill (3000): leftover.
        let o = p.fill_outcome(0, 4_000, 1_000);
        assert!(o.leftover() && !o.continues && o.rest == 1_000);
        // Exact.
        let o = p.fill_outcome(0, 9_000, 3_000);
        assert!(!o.continues && !o.leftover() && o.fee_output_index() == 2);
        assert_eq!(p.row(0).buyback_floor(3_000), 12_000);
    }

    #[test]
    #[should_panic(expected = "exceeds")]
    fn fill_outcome_refuses_more_than_remaining() {
        params().fill_outcome(0, 25_000, 10_000);
    }

    #[test]
    fn digest_covers_every_row() {
        let a = params();
        let mut b = a;
        b.rows[3].min_size = 1;
        assert_ne!(a.digest(), b.digest());
        let mut c = a;
        c.cutoff_height += 1;
        assert_ne!(a.digest(), c.digest());
    }
}
