use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_offer::derived_swaption_offer::SwaptionOfferWitness;
use crate::programs::swaption_lending_v5::PositionParametersV5;
use crate::programs::swaption_offer::{OfferParameters, WitnessOfferTerms};

/// The covenant's `FillArgs`: row selector (hi, lo), n, buyback_amount,
/// borrower_nft, borrower_payout_hash, last_look_hash, last_look_height.
pub type WitnessFillArgs = ((bool, bool), u64, u64, [u8; 32], [u8; 32], [u8; 32], u32);

#[derive(Debug, Clone, Copy)]
pub enum WitnessBranchOffer {
    Fill {
        terms: WitnessOfferTerms,
        remaining: u64,
        args: WitnessFillArgs,
    },
    Cancel {
        terms: WitnessOfferTerms,
        remaining: u64,
    },
    Expire {
        terms: WitnessOfferTerms,
        remaining: u64,
    },
}

impl WitnessBranchOffer {
    /// The fill witness for `position` on `row` of `offer` holding `remaining`.
    pub fn fill(offer: &OfferParameters, remaining: u64, row: u8, position: &PositionParametersV5) -> Self {
        assert!(usize::from(row) < crate::programs::swaption_offer::OFFER_ROWS, "row out of range");
        let selector = (row & 2 != 0, row & 1 != 0);
        WitnessBranchOffer::Fill {
            terms: offer.witness_terms(),
            remaining,
            args: (
                selector,
                position.terms.collateral_amount,
                position.terms.buyback_amount,
                position.borrower_nft_asset_id.into_inner().0,
                position.borrower_payout_script_hash,
                position.last_look_script_hash,
                position.last_look_height,
            ),
        }
    }

    pub fn build_witness(&self) -> Box<SwaptionOfferWitness> {
        let path = match self {
            WitnessBranchOffer::Fill { terms, remaining, args } => Left((*terms, *remaining, *args)),
            WitnessBranchOffer::Cancel { terms, remaining } => Right(Left((*terms, *remaining))),
            WitnessBranchOffer::Expire { terms, remaining } => Right(Right((*terms, *remaining))),
        };
        Box::new(SwaptionOfferWitness { path })
    }
}
